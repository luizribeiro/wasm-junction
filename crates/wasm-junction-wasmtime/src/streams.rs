use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use wasm_junction_core::{ChannelDirection, ImportDispatcher, InputStream, StreamHandle};
use wasmtime::component::{
    Destination, StreamAny, StreamProducer, StreamReader, StreamResult, VecBuffer,
};
use wasmtime::{AsContextMut, StoreContextMut};

use crate::engine::StoreData;

pub(crate) fn lower_stream(
    handle: StreamHandle,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamAny, wasmtime::Error> {
    let id = handle.id();
    let input =
        InputStream::try_from(handle).map_err(|error| wasmtime::Error::msg(error.to_string()))?;
    let imports = store.as_context().data().imports.clone();
    imports.channel_open(id, ChannelDirection::HostToGuest);
    let reader = StreamReader::new(
        store.as_context_mut(),
        CoreProducer {
            input: Some(input),
            id,
            imports,
            closed: false,
        },
    )?;
    reader.try_into_stream_any(store.as_context_mut())
}

struct CoreProducer {
    input: Option<InputStream>,
    id: u64,
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
                .channel_close(self.id, ChannelDirection::HostToGuest);
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
}
