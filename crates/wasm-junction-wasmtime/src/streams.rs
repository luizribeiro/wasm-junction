use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use wasm_junction_core::{
    ChannelDirection, ImportDispatcher, InputStream, InvocationId, OutputStream,
    OutputStreamWriter, StreamHandle,
};
use wasmtime::component::{
    Destination, Source, StreamAny, StreamConsumer, StreamProducer, StreamReader, StreamResult,
    VecBuffer,
};
use wasmtime::{AsContextMut, StoreContextMut};

use crate::engine::StoreData;

pub(crate) type ActiveStreams = Arc<Mutex<HashMap<u64, OutputStreamWriter>>>;

pub(crate) fn abort_streams(store: &StoreData) {
    let streams = lock_active(&store.active_streams)
        .drain()
        .collect::<Vec<_>>();
    for (id, writer) in streams {
        writer.abort();
        if let Some(invocation) = store.context.invocation_id() {
            store
                .imports
                .channel_close(invocation, id, ChannelDirection::GuestToHost);
        }
    }
}

pub(crate) fn lift_stream(
    stream: StreamAny,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamHandle, wasmtime::Error> {
    let reader = stream.try_into_stream_reader::<u8>()?;
    let (writer, output) = OutputStream::channel();
    let handle = StreamHandle::from(output);
    let id = handle.id();
    let imports = store.as_context().data().imports.clone();
    let invocation = invocation_id(store.as_context().data())?;
    imports.channel_open(invocation, id, ChannelDirection::GuestToHost);
    let active = store.as_context().data().active_streams.clone();
    lock_active(&active).insert(id, writer.clone());
    reader.pipe(
        store.as_context_mut(),
        CoreConsumer {
            writer: Some(writer),
            id,
            invocation,
            imports,
            active,
            closed: false,
        },
    )?;
    Ok(handle)
}

pub(crate) fn lower_stream(
    handle: StreamHandle,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamAny, wasmtime::Error> {
    let id = handle.id();
    let input =
        InputStream::try_from(handle).map_err(|error| wasmtime::Error::msg(error.to_string()))?;
    let imports = store.as_context().data().imports.clone();
    let invocation = invocation_id(store.as_context().data())?;
    imports.channel_open(invocation, id, ChannelDirection::HostToGuest);
    let reader = StreamReader::new(
        store.as_context_mut(),
        CoreProducer {
            input: Some(input),
            id,
            invocation,
            imports,
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
    closed: bool,
}

impl CoreProducer {
    fn close(&mut self) {
        if let Some(input) = self.input.take() {
            input.close_reader();
        }
        if !self.closed {
            self.closed = true;
            self.imports
                .channel_close(self.invocation, self.id, ChannelDirection::HostToGuest);
        }
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
    closed: bool,
}

impl CoreConsumer {
    fn close(&mut self) {
        let was_active = lock_active(&self.active).remove(&self.id).is_some();
        self.writer.take();
        if !self.closed {
            self.closed = true;
            if was_active {
                self.imports
                    .channel_close(self.invocation, self.id, ChannelDirection::GuestToHost);
            }
        }
    }
}

fn invocation_id(store: &StoreData) -> Result<InvocationId, wasmtime::Error> {
    store
        .context
        .invocation_id()
        .ok_or_else(|| wasmtime::Error::msg("stream has no invocation id"))
}

fn lock_active(active: &ActiveStreams) -> MutexGuard<'_, HashMap<u64, OutputStreamWriter>> {
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
