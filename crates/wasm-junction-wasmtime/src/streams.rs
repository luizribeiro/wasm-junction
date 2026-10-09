use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use wasm_junction_core::{
    ChannelDirection, EngineEvent, ImportDispatcher, InputStream, InvocationId, OutputStream,
    OutputStreamWriter, StreamHandle, Val,
};
use wasmtime::component::{
    Destination, Source, StreamAny, StreamConsumer, StreamProducer, StreamReader, StreamResult,
    VecBuffer,
};
use wasmtime::{AsContextMut, StoreContextMut};

use crate::engine::StoreData;

mod typed;

pub(crate) use typed::{lift_stream, lower_typed_stream, recover_exported_stream};
pub(crate) type ActiveStreams = Arc<Mutex<HashMap<u64, (ActiveWriter, ChannelDirection)>>>;

pub(crate) enum ActiveWriter {
    Bytes(OutputStreamWriter),
    Values(OutputStreamWriter<Val>),
}

impl ActiveWriter {
    fn abort(&self) {
        match self {
            Self::Bytes(writer) => writer.abort(),
            Self::Values(writer) => writer.abort(),
        }
    }
}

pub(crate) fn abort_streams(store: &StoreData) {
    let streams = lock_active(&store.active_streams)
        .drain()
        .collect::<Vec<_>>();
    for (id, (writer, direction)) in streams {
        writer.abort();
        if let Some(invocation) = store.context.invocation_id() {
            store.imports.emit(EngineEvent::ChannelClose {
                invocation,
                stream: id,
                direction,
            });
        }
    }
}

pub(crate) fn lift_stream_with_direction(
    stream: StreamAny,
    mut store: impl AsContextMut<Data = StoreData>,
    direction: ChannelDirection,
) -> Result<StreamHandle, wasmtime::Error> {
    let reader = stream.try_into_stream_reader::<u8>()?;
    let (writer, output) = OutputStream::channel();
    let handle = StreamHandle::from(output);
    let id = handle.id();
    let imports = store.as_context().data().imports.clone();
    let invocation = invocation_id(store.as_context().data())?;
    imports.emit(EngineEvent::ChannelOpen {
        invocation,
        stream: id,
        direction,
    });
    let active = store.as_context().data().active_streams.clone();
    lock_active(&active).insert(id, (ActiveWriter::Bytes(writer.clone()), direction));
    reader.pipe(
        store.as_context_mut(),
        CoreConsumer {
            writer: Some(writer),
            id,
            invocation,
            imports,
            active,
            direction,
            closed: false,
        },
    )?;
    Ok(handle)
}

pub(crate) fn lower_stream(
    handle: StreamHandle,
    store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamAny, wasmtime::Error> {
    lower_stream_with_direction(handle, store, Some(ChannelDirection::HostToGuest))
}

pub(crate) fn lower_stream_with_direction(
    handle: StreamHandle,
    mut store: impl AsContextMut<Data = StoreData>,
    direction: Option<ChannelDirection>,
) -> Result<StreamAny, wasmtime::Error> {
    let id = handle.id();
    let input =
        InputStream::try_from(handle).map_err(|error| wasmtime::Error::msg(error.to_string()))?;
    let imports = store.as_context().data().imports.clone();
    let invocation = invocation_id(store.as_context().data())?;
    if let Some(direction) = direction {
        imports.emit(EngineEvent::ChannelOpen {
            invocation,
            stream: id,
            direction,
        });
    }
    let reader = StreamReader::new(
        store.as_context_mut(),
        CoreProducer {
            input: Some(input),
            id,
            invocation,
            imports,
            direction,
            closed: false,
        },
    )?;
    reader.try_into_stream_any(store.as_context_mut())
}

struct CoreProducer {
    input: Option<InputStream>,
    id: u64,
    invocation: InvocationId,
    imports: Arc<dyn ImportDispatcher>,
    direction: Option<ChannelDirection>,
    closed: bool,
}

impl CoreProducer {
    fn close(&mut self) {
        if let Some(input) = self.input.take() {
            input.close_reader();
        }
        close_channel(
            &mut self.closed,
            &self.imports,
            self.invocation,
            self.id,
            self.direction,
            None,
        );
    }
}

impl Drop for CoreProducer {
    fn drop(&mut self) {
        self.close();
    }
}

impl StreamProducer<StoreData> for CoreProducer {
    type Item = u8;
    type Buffer = VecBuffer<u8>;

    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        mut store: StoreContextMut<'a, StoreData>,
        mut destination: Destination<'a, Self::Item, Self::Buffer>,
        finish: bool,
    ) -> Poll<Result<StreamResult, wasmtime::Error>> {
        let this = self.get_mut();
        if finish {
            this.close();
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if destination.remaining(store.as_context_mut()) == Some(0) {
            return Poll::Ready(Ok(StreamResult::Completed));
        }
        let polled = {
            let Some(input) = &mut this.input else {
                return Poll::Ready(Ok(StreamResult::Dropped));
            };
            let future = input.read();
            let mut future = std::pin::pin!(future);
            future.as_mut().poll(context)
        };
        match polled {
            Poll::Ready(Ok(Some(bytes))) => {
                destination.set_buffer(bytes.into());
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(Ok(None)) => {
                this.close();
                Poll::Ready(Ok(StreamResult::Dropped))
            }
            Poll::Ready(Err(error)) => {
                this.close();
                Poll::Ready(Err(wasmtime::Error::msg(error.to_string())))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn try_into(mut me: Pin<Box<Self>>, ty: TypeId) -> Result<Box<dyn Any>, Pin<Box<Self>>> {
        if ty != TypeId::of::<StreamHandle>() {
            return Err(me);
        }
        let Some(input) = me.as_mut().get_mut().input.take() else {
            return Err(me);
        };
        let handle = input.into_handle();
        me.as_mut().get_mut().close();
        Ok(Box::new(handle))
    }
}

struct CoreConsumer {
    writer: Option<OutputStreamWriter>,
    id: u64,
    invocation: InvocationId,
    imports: Arc<dyn ImportDispatcher>,
    active: ActiveStreams,
    direction: ChannelDirection,
    closed: bool,
}

impl CoreConsumer {
    fn close(&mut self) {
        self.writer.take();
        close_channel(
            &mut self.closed,
            &self.imports,
            self.invocation,
            self.id,
            Some(self.direction),
            Some(&self.active),
        );
    }
}

fn close_channel(
    closed: &mut bool,
    imports: &Arc<dyn ImportDispatcher>,
    invocation: InvocationId,
    id: u64,
    direction: Option<ChannelDirection>,
    active: Option<&ActiveStreams>,
) {
    let was_active = active.is_none_or(|active| lock_active(active).remove(&id).is_some());
    if std::mem::replace(closed, true) || !was_active {
        return;
    }
    if let Some(direction) = direction {
        imports.emit(EngineEvent::ChannelClose {
            invocation,
            stream: id,
            direction,
        });
    }
}

pub(super) fn invocation_id(store: &StoreData) -> Result<InvocationId, wasmtime::Error> {
    store
        .context
        .invocation_id()
        .ok_or_else(|| wasmtime::Error::msg("stream has no invocation id"))
}

pub(super) fn lock_active(
    active: &ActiveStreams,
) -> MutexGuard<'_, HashMap<u64, (ActiveWriter, ChannelDirection)>> {
    match active.lock() {
        Ok(streams) => streams,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Drop for CoreConsumer {
    fn drop(&mut self) {
        self.close();
    }
}

impl StreamConsumer<StoreData> for CoreConsumer {
    type Item = u8;

    fn poll_consume(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        store: StoreContextMut<'_, StoreData>,
        source: Source<'_, Self::Item>,
        finish: bool,
    ) -> Poll<Result<StreamResult, wasmtime::Error>> {
        let this = self.get_mut();
        let mut source = source.as_direct(store);
        let bytes = source.remaining().to_vec();
        source.mark_read(bytes.len());
        if !bytes.is_empty() {
            let polled = {
                let Some(writer) = &this.writer else {
                    return Poll::Ready(Ok(StreamResult::Dropped));
                };
                let future = writer.write(bytes);
                let mut future = std::pin::pin!(future);
                future.as_mut().poll(context)
            };
            match polled {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(_)) => {
                    this.close();
                    return Poll::Ready(Ok(StreamResult::Dropped));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        if finish {
            this.close();
            Poll::Ready(Ok(StreamResult::Dropped))
        } else {
            Poll::Ready(Ok(StreamResult::Completed))
        }
    }
}
