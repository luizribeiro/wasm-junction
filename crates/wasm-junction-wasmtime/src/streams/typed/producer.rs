use std::any::{Any, TypeId};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use wasm_junction_core::{
    ChannelDirection, EngineEvent, ImportDispatcher, InputStream, InvocationId, StreamHandle, Val,
};
use wasmtime::component::{
    Destination, StreamAny, StreamProducer, StreamReader, StreamResult, Type, VecBuffer,
};
use wasmtime::{AsContextMut, StoreContextMut};

use super::super::{close_channel, invocation_id};
use super::restore_owned_resources;
use crate::engine::StoreData;
use crate::stream_values::StreamValue;

pub(super) fn lower<T: StreamValue>(
    handle: StreamHandle,
    item_type: Type,
    mut store: StoreContextMut<'_, StoreData>,
) -> Result<StreamAny, wasmtime::Error> {
    let reader = lower_reader::<T>(handle, Some(item_type), store.as_context_mut())?;
    reader.try_into_stream_any(store.as_context_mut())
}

pub(super) fn lower_reader<T: StreamValue>(
    handle: StreamHandle,
    item_type: Option<Type>,
    mut store: StoreContextMut<'_, StoreData>,
) -> Result<StreamReader<T>, wasmtime::Error> {
    let id = handle.id();
    let input = InputStream::<Val>::__from_handle_with(handle, Ok)
        .map_err(|error| wasmtime::Error::msg(error.to_string()))?;
    let imports = store.data().imports.clone();
    let invocation = invocation_id(store.data())?;
    imports.emit(EngineEvent::ChannelOpen {
        invocation,
        stream: id,
        direction: ChannelDirection::HostToGuest,
    });
    let reader = StreamReader::new(
        store.as_context_mut(),
        Producer::<T> {
            input: Some(input),
            item_type,
            id,
            invocation,
            imports,
            closed: false,
            marker: std::marker::PhantomData,
        },
    )?;
    Ok(reader)
}

struct Producer<T> {
    input: Option<InputStream<Val>>,
    item_type: Option<Type>,
    id: u64,
    invocation: InvocationId,
    imports: Arc<dyn ImportDispatcher>,
    closed: bool,
    marker: std::marker::PhantomData<T>,
}

impl<T> Producer<T> {
    fn close(&mut self) {
        if let Some(input) = self.input.take() {
            input.close_reader();
        }
        close_channel(
            &mut self.closed,
            &self.imports,
            self.invocation,
            self.id,
            Some(ChannelDirection::HostToGuest),
            None,
        );
    }
}

impl<T> Drop for Producer<T> {
    fn drop(&mut self) {
        self.close();
    }
}

impl<T: StreamValue> StreamProducer<StoreData> for Producer<T> {
    type Item = T;
    type Buffer = VecBuffer<T>;

    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        mut store: StoreContextMut<'a, StoreData>,
        mut destination: Destination<'a, T, VecBuffer<T>>,
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
            std::pin::pin!(future).poll(context)
        };
        match polled {
            Poll::Ready(Ok(Some(values))) => {
                let mut converted = Vec::with_capacity(values.len());
                let mut lowered = Vec::with_capacity(values.len());
                for value in values {
                    match T::from_val(Some(value.clone()), this.item_type.as_ref(), &mut store) {
                        Ok(item) => {
                            converted.push(item);
                            lowered.push(value);
                        }
                        Err(error) => {
                            restore_owned_resources(&lowered, store.data_mut());
                            this.close();
                            return Poll::Ready(Err(error));
                        }
                    }
                }
                destination.set_buffer(converted.into());
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
