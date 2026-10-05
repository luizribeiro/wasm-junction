use std::any::Any;
use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{self, Debug, Display};
use std::future::poll_fn;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Poll, Waker};

use crate::{CallError, CompiledComponent, FromVal, HostBound, ToVal, TypeError, Val};

static NEXT_STREAM_ID: AtomicU64 = AtomicU64::new(1);

struct StreamState {
    chunks: VecDeque<StreamChunk>,
    end: StreamEnd,
    reader_taken: bool,
    reader_waker: Option<Waker>,
    writers: usize,
}

enum StreamChunk {
    Bytes(Vec<u8>),
    Values(Vec<Val>),
}

#[cfg(not(target_arch = "wasm32"))]
type ItemMap = Arc<dyn Fn(Val) -> Val + Send + Sync>;
#[cfg(target_arch = "wasm32")]
type ItemMap = Arc<dyn Fn(Val) -> Val>;
#[cfg(not(target_arch = "wasm32"))]
type ItemFilter = Arc<dyn Fn(&Val) -> bool + Send + Sync>;
#[cfg(target_arch = "wasm32")]
type ItemFilter = Arc<dyn Fn(&Val) -> bool>;
#[cfg(not(target_arch = "wasm32"))]
type ChunkMap = Arc<dyn Fn(Vec<u8>) -> Vec<u8> + Send + Sync>;
#[cfg(target_arch = "wasm32")]
type ChunkMap = Arc<dyn Fn(Vec<u8>) -> Vec<u8>>;

#[derive(Clone)]
enum Transform {
    MapItems(ItemMap),
    FilterItems(ItemFilter),
    MapChunks(ChunkMap),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamEnd {
    Open,
    Closed,
    ReaderClosed,
    Aborted,
    Abandoned,
}

/// An opaque, cloneable reference to a WIT value stream.
#[derive(Clone)]
pub struct StreamHandle {
    id: u64,
    state: Arc<Mutex<StreamState>>,
    generation: Option<Arc<dyn CompiledComponent>>,
    transforms: Vec<Transform>,
}

impl StreamHandle {
    fn new(chunks: VecDeque<StreamChunk>, end: StreamEnd, writers: usize) -> Self {
        Self {
            id: NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed),
            state: Arc::new(Mutex::new(StreamState {
                chunks,
                end,
                reader_taken: false,
                reader_waker: None,
                writers,
            })),
            generation: None,
            transforms: Vec::new(),
        }
    }

    /// Returns the id used to correlate this stream's lifecycle events.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Replaces this handle with an empty stream and returns its previous value.
    #[must_use]
    pub fn take(&mut self) -> Self {
        std::mem::replace(self, Self::new(VecDeque::new(), StreamEnd::Closed, 0))
    }

    /// Maps each engine-neutral item when the stream is read.
    #[must_use]
    pub fn map_items(mut self, map: impl Fn(Val) -> Val + HostBound + 'static) -> Self {
        self.transforms.push(Transform::MapItems(Arc::new(map)));
        self
    }

    /// Keeps engine-neutral items that satisfy `predicate` when the stream is read.
    ///
    /// ```
    /// use wasm_junction_core::{InputStream, OutputStream, StreamHandle, Val};
    ///
    /// let stream = OutputStream::from_items(["public".to_owned(), ".hidden".to_owned()]);
    /// let visible = StreamHandle::from(stream).filter_items(|item| {
    ///     !matches!(item, Val::String(name) if name.starts_with('.'))
    /// });
    /// let _reader = InputStream::<String>::from_handle(visible)?;
    /// # Ok::<(), wasm_junction_core::StreamError>(())
    /// ```
    #[must_use]
    pub fn filter_items(mut self, predicate: impl Fn(&Val) -> bool + HostBound + 'static) -> Self {
        self.transforms
            .push(Transform::FilterItems(Arc::new(predicate)));
        self
    }

    /// Maps each byte chunk when the stream is read.
    #[must_use]
    pub fn map_chunks(mut self, map: impl Fn(Vec<u8>) -> Vec<u8> + HostBound + 'static) -> Self {
        self.transforms.push(Transform::MapChunks(Arc::new(map)));
        self
    }

    /// Keeps the component generation that returned this stream alive while the handle exists.
    ///
    /// # Errors
    ///
    /// Returns an error if another component generation already owns the stream.
    #[doc(hidden)]
    pub fn pin_generation(
        &mut self,
        generation: Arc<dyn CompiledComponent>,
    ) -> Result<(), CallError> {
        match &self.generation {
            None => {
                self.generation = Some(generation);
                Ok(())
            }
            Some(current) if Arc::ptr_eq(current, &generation) => Ok(()),
            Some(_) => Err(CallError::refused(
                "stream belongs to another component generation",
            )),
        }
    }

    fn state(&self) -> MutexGuard<'_, StreamState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Debug for StreamHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for StreamHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for StreamHandle {}

/// A stream consumed by a host provider.
pub struct InputStream<T = u8> {
    handle: StreamHandle,
    finished: bool,
    item: PhantomData<T>,
}

impl<T: FromVal + 'static> InputStream<T> {
    /// Claims the single reader for an opaque stream handle.
    ///
    /// # Errors
    ///
    /// Returns an error if another reader already claimed the stream.
    pub fn from_handle(handle: StreamHandle) -> Result<Self, StreamError> {
        {
            let mut state = handle.state();
            if state.reader_taken {
                return Err(StreamError::already_read());
            }
            state.reader_taken = true;
        }
        Ok(Self {
            handle,
            finished: false,
            item: PhantomData,
        })
    }

    /// Reads the next available chunk, or `None` after a clean end of stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the stream cannot be read.
    pub async fn read(&mut self) -> Result<Option<Vec<T>>, StreamError> {
        let result = poll_fn(|context| {
            loop {
                let mut state = self.handle.state();
                if let Some(chunk) = state.chunks.pop_front() {
                    drop(state);
                    let chunk = match apply_transforms(chunk, &self.handle.transforms) {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => continue,
                        Err(error) => return Poll::Ready(Err(error)),
                    };
                    return Poll::Ready(decode_chunk(chunk).map(Some));
                }
                return match state.end {
                    StreamEnd::Open => {
                        state.reader_waker = Some(context.waker().clone());
                        Poll::Pending
                    }
                    StreamEnd::Closed | StreamEnd::ReaderClosed | StreamEnd::Abandoned => {
                        Poll::Ready(Ok(None))
                    }
                    StreamEnd::Aborted => Poll::Ready(Err(StreamError::aborted())),
                };
            }
        })
        .await;
        self.finished = !matches!(result, Ok(Some(_)));
        result
    }

    /// Collects every remaining chunk into one item vector.
    ///
    /// # Errors
    ///
    /// Returns an error rather than silently truncating a stream whose invocation ended early.
    pub async fn read_all(mut self) -> Result<Vec<T>, StreamError> {
        let mut items = Vec::new();
        while let Some(chunk) = self.read().await? {
            items.extend(chunk);
        }
        Ok(items)
    }
    /// Releases this reader back into an opaque handle for an engine boundary transfer.
    #[doc(hidden)]
    #[must_use]
    pub fn into_handle(mut self) -> StreamHandle {
        self.handle.state().reader_taken = false;
        self.finished = true;
        self.handle.clone()
    }

    /// Closes the reader while preserving the reason for subsequent writer failures.
    #[doc(hidden)]
    pub fn close_reader(mut self) {
        let mut state = self.handle.state();
        if state.end == StreamEnd::Open {
            state.end = StreamEnd::ReaderClosed;
            state.chunks.clear();
        }
        self.finished = true;
    }
}

impl<T> Drop for InputStream<T> {
    fn drop(&mut self) {
        if !self.finished {
            let mut state = self.handle.state();
            if state.end == StreamEnd::Open {
                state.end = StreamEnd::Abandoned;
                state.chunks.clear();
            }
        }
    }
}

impl TryFrom<StreamHandle> for InputStream<u8> {
    type Error = StreamError;

    fn try_from(handle: StreamHandle) -> Result<Self, Self::Error> {
        Self::from_handle(handle)
    }
}

impl TryFrom<Val> for InputStream<u8> {
    type Error = TypeError;

    fn try_from(value: Val) -> Result<Self, Self::Error> {
        let Val::Stream(handle) = value else {
            return Err(TypeError::new("expected stream"));
        };
        Self::from_handle(handle).map_err(|error| TypeError::new(error.to_string()))
    }
}

/// A stream produced by a host provider.
pub struct OutputStream<T = u8>(StreamHandle, PhantomData<T>);

impl OutputStream<u8> {
    /// Creates a stream containing one byte chunk and a clean end marker.
    #[must_use]
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(
            StreamHandle::new(
                VecDeque::from([StreamChunk::Bytes(bytes.into())]),
                StreamEnd::Closed,
                0,
            ),
            PhantomData,
        )
    }
}

impl<T: ToVal + 'static> OutputStream<T> {
    /// Creates a stream containing the supplied items and a clean end marker.
    #[must_use]
    pub fn from_items(items: impl IntoIterator<Item = T>) -> Self {
        Self(
            StreamHandle::new(
                VecDeque::from([encode_chunk(items.into_iter().collect())]),
                StreamEnd::Closed,
                0,
            ),
            PhantomData,
        )
    }

    /// Creates an open stream and the writer used to produce its chunks.
    ///
    /// The channel is unbounded. A slow or absent reader leaves every written chunk buffered in
    /// memory until it is read or the reader is dropped.
    #[must_use]
    pub fn channel() -> (OutputStreamWriter<T>, Self) {
        let handle = StreamHandle::new(VecDeque::new(), StreamEnd::Open, 1);
        (
            OutputStreamWriter(handle.clone(), PhantomData),
            Self(handle, PhantomData),
        )
    }
}

/// A cloneable producer for an [`OutputStream`].
pub struct OutputStreamWriter<T = u8>(StreamHandle, PhantomData<T>);

impl<T> Clone for OutputStreamWriter<T> {
    fn clone(&self) -> Self {
        self.0.state().writers += 1;
        Self(self.0.clone(), PhantomData)
    }
}

impl<T: ToVal + 'static> OutputStreamWriter<T> {
    /// Appends one chunk without blocking on the reader.
    ///
    /// The channel is unbounded, so a slow or absent reader leaves every written chunk buffered
    /// in memory until it is read or the reader is dropped.
    ///
    /// # Errors
    ///
    /// Returns an error if the reader abandoned the stream or the stream was aborted.
    pub async fn write(&self, chunk: impl Into<Vec<T>>) -> Result<(), StreamError> {
        let (result, waker) = {
            let mut state = self.0.state();
            match state.end {
                StreamEnd::Open => {
                    state.chunks.push_back(encode_chunk(chunk.into()));
                    (Ok(()), state.reader_waker.take())
                }
                StreamEnd::Closed => (Err(StreamError::closed()), None),
                StreamEnd::ReaderClosed => (Err(StreamError::reader_closed()), None),
                StreamEnd::Aborted => (Err(StreamError::aborted()), None),
                StreamEnd::Abandoned => (Err(StreamError::abandoned()), None),
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        std::future::ready(result).await
    }

    /// Marks the stream as cut off and wakes its reader.
    pub fn abort(&self) {
        let waker = {
            let mut state = self.0.state();
            if state.end == StreamEnd::Open {
                state.end = StreamEnd::Aborted;
                state.reader_waker.take()
            } else {
                None
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> Drop for OutputStreamWriter<T> {
    fn drop(&mut self) {
        let waker = {
            let mut state = self.0.state();
            state.writers -= 1;
            if state.writers == 0 && state.end == StreamEnd::Open {
                state.end = StreamEnd::Closed;
                state.reader_waker.take()
            } else {
                None
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> From<OutputStream<T>> for StreamHandle {
    fn from(stream: OutputStream<T>) -> Self {
        stream.0
    }
}

impl<T> From<OutputStream<T>> for Val {
    fn from(stream: OutputStream<T>) -> Self {
        Self::Stream(stream.into())
    }
}

fn encode_chunk<T: ToVal + 'static>(items: Vec<T>) -> StreamChunk {
    let erased = &items as &dyn Any;
    if let Some(bytes) = erased.downcast_ref::<Vec<u8>>() {
        StreamChunk::Bytes(bytes.clone())
    } else {
        StreamChunk::Values(items.into_iter().map(ToVal::to_val).collect())
    }
}

fn apply_transforms(
    mut chunk: StreamChunk,
    transforms: &[Transform],
) -> Result<Option<StreamChunk>, StreamError> {
    for transform in transforms {
        chunk = match (transform, chunk) {
            (Transform::MapChunks(map), StreamChunk::Bytes(bytes)) => {
                StreamChunk::Bytes(map(bytes))
            }
            (Transform::MapChunks(_), StreamChunk::Values(_)) => {
                return Err(StreamError::shape("expected byte stream"));
            }
            (Transform::MapItems(map), chunk) => StreamChunk::Values(
                chunk
                    .into_values()
                    .into_iter()
                    .map(|item| map(item))
                    .collect(),
            ),
            (Transform::FilterItems(predicate), chunk) => StreamChunk::Values(
                chunk
                    .into_values()
                    .into_iter()
                    .filter(|item| predicate(item))
                    .collect(),
            ),
        };
    }
    Ok((!chunk.is_empty()).then_some(chunk))
}

impl StreamChunk {
    fn into_values(self) -> Vec<Val> {
        match self {
            Self::Bytes(bytes) => bytes.into_iter().map(Val::U8).collect(),
            Self::Values(values) => values,
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Bytes(bytes) => bytes.is_empty(),
            Self::Values(values) => values.is_empty(),
        }
    }
}

fn decode_chunk<T: FromVal + 'static>(chunk: StreamChunk) -> Result<Vec<T>, StreamError> {
    match chunk {
        StreamChunk::Bytes(bytes) => (Box::new(bytes) as Box<dyn Any>)
            .downcast::<Vec<T>>()
            .map(|items| *items)
            .map_err(|_| StreamError::shape("expected value stream")),
        StreamChunk::Values(values) => values
            .into_iter()
            .map(FromVal::from_val)
            .collect::<Result<_, _>>()
            .map_err(|error| StreamError::shape(error.to_string())),
    }
}

/// A failure to read from or write to a stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamError(String);

impl StreamError {
    fn aborted() -> Self {
        Self("stream was aborted when its invocation ended".to_owned())
    }

    fn already_read() -> Self {
        Self("stream already has a reader".to_owned())
    }

    fn closed() -> Self {
        Self("stream is closed".to_owned())
    }

    fn reader_closed() -> Self {
        Self("stream reader is closed".to_owned())
    }

    fn abandoned() -> Self {
        Self("stream reader abandoned the stream".to_owned())
    }

    fn shape(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl Display for StreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for StreamError {}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Waker};

    use super::*;

    #[derive(Debug, PartialEq)]
    struct Note {
        text: String,
    }

    impl From<Note> for Val {
        fn from(note: Note) -> Self {
            Self::Record(vec![("text".to_owned(), note.text.into())])
        }
    }

    impl TryFrom<Val> for Note {
        type Error = TypeError;

        fn try_from(value: Val) -> Result<Self, Self::Error> {
            let Val::Record(fields) = value else {
                return Err(TypeError::new("expected note"));
            };
            let [(name, value)] = <[(String, Val); 1]>::try_from(fields)
                .map_err(|_| TypeError::new("expected note"))?;
            if name != "text" {
                return Err(TypeError::new("expected note text"));
            }
            Ok(Self {
                text: String::try_from(value)?,
            })
        }
    }

    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("future unexpectedly suspended"),
        }
    }

    #[test]
    fn byte_stream_has_one_reader() {
        let value = Val::from(OutputStream::from_bytes(b"hello"));
        let mut input = InputStream::try_from(value.clone()).unwrap();
        assert_eq!(ready(input.read()).unwrap(), Some(b"hello".to_vec()));
        assert_eq!(ready(input.read()).unwrap(), None);

        let Err(error) = InputStream::try_from(value) else {
            panic!("cloned handle gained another reader");
        };
        assert_eq!(error.to_string(), "stream already has a reader");

        let handle = input.into_handle();
        assert!(InputStream::try_from(handle).is_ok());
    }

    #[test]
    fn value_stream_preserves_item_order_and_reports_wrong_shapes() {
        let output = OutputStream::from_items([
            Note {
                text: "first".to_owned(),
            },
            Note {
                text: "second".to_owned(),
            },
        ]);
        let mut input = InputStream::<Note>::from_handle(StreamHandle::from(output)).unwrap();
        assert_eq!(
            ready(input.read()).unwrap(),
            Some(vec![
                Note {
                    text: "first".to_owned()
                },
                Note {
                    text: "second".to_owned()
                }
            ])
        );
        assert_eq!(ready(input.read()).unwrap(), None);

        let output = OutputStream::from_items(["not a note".to_owned()]);
        let mut input = InputStream::<Note>::from_handle(StreamHandle::from(output)).unwrap();
        assert_eq!(
            ready(input.read()).unwrap_err().to_string(),
            "expected note"
        );
    }

    #[test]
    fn transforms_are_pull_based_and_preserve_end_and_abort() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let handle = StreamHandle::from(OutputStream::from_items([
            "first".to_owned(),
            "hidden".to_owned(),
            "last".to_owned(),
        ]))
        .map_items(move |item| {
            observed.fetch_add(1, Ordering::Relaxed);
            item
        })
        .filter_items(|item| item != &Val::String("hidden".to_owned()));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        let input = InputStream::<String>::from_handle(handle).unwrap();
        assert_eq!(
            ready(input.read_all()).unwrap(),
            ["first".to_owned(), "last".to_owned()]
        );
        assert_eq!(calls.load(Ordering::Relaxed), 3);

        let handle =
            StreamHandle::from(OutputStream::from_bytes(b"quiet")).map_chunks(|mut bytes| {
                bytes.make_ascii_uppercase();
                bytes
            });
        let input = InputStream::try_from(handle).unwrap();
        assert_eq!(ready(input.read_all()).unwrap(), b"QUIET");

        let handle = StreamHandle::from(OutputStream::from_bytes([1, 2])).map_items(|item| {
            let Val::U8(byte) = item else {
                panic!("byte stream exposed a non-byte item")
            };
            Val::U8(byte + 1)
        });
        let input = InputStream::try_from(handle).unwrap();
        assert_eq!(ready(input.read_all()).unwrap(), [2, 3]);

        let handle =
            StreamHandle::from(OutputStream::from_items([7_u32])).map_chunks(|bytes| bytes);
        let mut input = InputStream::<u32>::from_handle(handle).unwrap();
        assert_eq!(
            ready(input.read()).unwrap_err().to_string(),
            "expected byte stream"
        );

        let (writer, output) = OutputStream::<u32>::channel();
        let mut input =
            InputStream::<u32>::from_handle(StreamHandle::from(output).filter_items(|_| true))
                .unwrap();
        writer.abort();
        assert_eq!(ready(input.read()), Err(StreamError::aborted()));
    }

    #[test]
    fn channel_delivers_chunks_and_reports_close_or_abort() {
        let (writer, output) = OutputStream::channel();
        let second = writer.clone();
        let mut input = InputStream::try_from(StreamHandle::from(output)).unwrap();
        ready(writer.write(b"hello ")).unwrap();
        drop(writer);
        ready(second.write(b"world")).unwrap();
        assert_eq!(ready(input.read()).unwrap(), Some(b"hello ".to_vec()));
        assert_eq!(ready(input.read()).unwrap(), Some(b"world".to_vec()));
        drop(second);
        assert_eq!(ready(input.read()).unwrap(), None);

        let (writer, output) = OutputStream::channel();
        let input = InputStream::try_from(StreamHandle::from(output)).unwrap();
        drop(input);
        assert_eq!(ready(writer.write(b"late")), Err(StreamError::abandoned()));

        let (writer, output) = OutputStream::channel();
        InputStream::try_from(StreamHandle::from(output))
            .unwrap()
            .close_reader();
        assert_eq!(
            ready(writer.write(b"late")),
            Err(StreamError::reader_closed())
        );

        let (writer, output) = OutputStream::channel();
        let mut input = InputStream::try_from(StreamHandle::from(output)).unwrap();
        writer.abort();
        assert_eq!(ready(input.read()), Err(StreamError::aborted()));
        assert_eq!(ready(writer.write(b"late")), Err(StreamError::aborted()));
    }
}
