use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{self, Debug, Display};
use std::future::poll_fn;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::Poll;

use crate::{TypeError, Val};

static NEXT_STREAM_ID: AtomicU64 = AtomicU64::new(1);

struct StreamState {
    chunks: VecDeque<Vec<u8>>,
    reader_taken: bool,
}

/// An opaque, cloneable reference to a byte stream.
#[derive(Clone)]
pub struct StreamHandle {
    id: u64,
    state: Arc<Mutex<StreamState>>,
}

impl StreamHandle {
    fn new(chunks: VecDeque<Vec<u8>>) -> Self {
        Self {
            id: NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed),
            state: Arc::new(Mutex::new(StreamState {
                chunks,
                reader_taken: false,
            })),
        }
    }

    /// Returns the id used to correlate this stream's lifecycle events.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
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

/// A byte stream consumed by a host provider.
pub struct InputStream {
    handle: StreamHandle,
}

impl InputStream {
    /// Reads the next available chunk, or `None` after a clean end of stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the stream cannot be read.
    pub async fn read(&mut self) -> Result<Option<Vec<u8>>, StreamError> {
        poll_fn(|_| Poll::Ready(Ok(self.handle.state().chunks.pop_front()))).await
    }

    /// Collects every remaining chunk into one byte vector.
    ///
    /// # Errors
    ///
    /// Returns an error rather than silently truncating a stream whose invocation ended early.
    pub async fn read_all(mut self) -> Result<Vec<u8>, StreamError> {
        let mut bytes = Vec::new();
        while let Some(chunk) = self.read().await? {
            bytes.extend(chunk);
        }
        Ok(bytes)
    }
}

impl TryFrom<StreamHandle> for InputStream {
    type Error = StreamError;

    fn try_from(handle: StreamHandle) -> Result<Self, Self::Error> {
        {
            let mut state = handle.state();
            if state.reader_taken {
                return Err(StreamError::already_read());
            }
            state.reader_taken = true;
        }
        Ok(Self { handle })
    }
}

impl TryFrom<Val> for InputStream {
    type Error = TypeError;

    fn try_from(value: Val) -> Result<Self, Self::Error> {
        let Val::Stream(handle) = value else {
            return Err(TypeError::new("expected stream<u8>"));
        };
        Self::try_from(handle).map_err(|error| TypeError::new(error.to_string()))
    }
}

/// A byte stream produced by a host provider.
pub struct OutputStream(StreamHandle);

impl OutputStream {
    /// Creates a stream containing one byte chunk and a clean end marker.
    #[must_use]
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(StreamHandle::new(VecDeque::from([bytes.into()])))
    }
}

impl From<OutputStream> for StreamHandle {
    fn from(stream: OutputStream) -> Self {
        stream.0
    }
}

impl From<OutputStream> for Val {
    fn from(stream: OutputStream) -> Self {
        Self::Stream(stream.into())
    }
}

/// A failure to read from or write to a byte stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamError(&'static str);

impl StreamError {
    const fn already_read() -> Self {
        Self("stream already has a reader")
    }
}

impl Display for StreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for StreamError {}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Waker};

    use super::*;

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
    }
}
