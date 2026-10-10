use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use wasm_junction_core::{
    ChannelDirection, EngineEvent, ImportDispatcher, InvocationId, OutputStream,
    OutputStreamWriter, StreamHandle, Val,
};
use wasmtime::component::{Source, StreamAny, StreamConsumer, StreamResult, Type};
use wasmtime::{AsContextMut, StoreContextMut};

use super::super::{ActiveStreams, ActiveWriter, close_channel, invocation_id, lock_active};
use super::restore_owned_resources;
use crate::engine::StoreData;
use crate::stream_values::StreamValue;

pub(super) fn lift<T: StreamValue>(
    stream: StreamAny,
    item_type: Type,
    store: StoreContextMut<'_, StoreData>,
) -> Result<StreamHandle, wasmtime::Error> {
    let reader = stream.try_into_stream_reader::<T>()?;
    lift_reader(
        reader,
        Some(item_type),
        store,
        ChannelDirection::GuestToHost,
    )
}

pub(super) fn lift_reader<T: StreamValue>(
    reader: wasmtime::component::StreamReader<T>,
    item_type: Option<Type>,
    mut store: StoreContextMut<'_, StoreData>,
    direction: ChannelDirection,
) -> Result<StreamHandle, wasmtime::Error> {
    let (writer, output) = OutputStream::<Val>::channel();
    let handle = output.__into_handle_with(Ok);
    let id = handle.id();
    let imports = store.data().imports.clone();
    let invocation = invocation_id(store.data())?;
    imports.emit(EngineEvent::ChannelOpen {
        invocation,
        stream: id,
        direction,
    });
    let active = store.data().active_streams.clone();
    lock_active(&active).insert(id, (ActiveWriter::Values(writer.clone()), direction));
    reader.pipe(
        store.as_context_mut(),
        Consumer::<T> {
            writer: Some(writer),
            item_type,
            id,
            invocation,
            imports,
            active,
            direction,
            closed: false,
            marker: std::marker::PhantomData,
        },
    )?;
    Ok(handle)
}

struct Consumer<T> {
    writer: Option<OutputStreamWriter<Val>>,
    item_type: Option<Type>,
    id: u64,
    invocation: InvocationId,
    imports: Arc<dyn ImportDispatcher>,
    active: ActiveStreams,
    direction: ChannelDirection,
    closed: bool,
    marker: std::marker::PhantomData<T>,
}

impl<T> Consumer<T> {
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

impl<T> Drop for Consumer<T> {
    fn drop(&mut self) {
        self.close();
    }
}

impl<T: StreamValue> StreamConsumer<StoreData> for Consumer<T> {
    type Item = T;
    fn poll_consume(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        mut store: StoreContextMut<'_, StoreData>,
        mut source: Source<'_, T>,
        finish: bool,
    ) -> Poll<Result<StreamResult, wasmtime::Error>> {
        let this = self.get_mut();
        let mut items = Vec::with_capacity(source.remaining(store.as_context_mut()));
        source.read(store.as_context_mut(), &mut items)?;
        let mut values = Vec::with_capacity(items.len());
        for item in items {
            match item
                .into_val(this.item_type.as_ref(), &mut store)
                .and_then(|value| {
                    value.ok_or_else(|| wasmtime::Error::msg("stream item cannot be unit"))
                }) {
                Ok(value) => values.push(value),
                Err(error) => {
                    restore_owned_resources(&values, store.data_mut());
                    if let Some(writer) = &this.writer {
                        writer.abort();
                    }
                    this.close();
                    return Poll::Ready(Err(error));
                }
            }
        }
        if !values.is_empty() {
            let Some(writer) = this.writer.clone() else {
                restore_owned_resources(&values, store.data_mut());
                return Poll::Ready(Ok(StreamResult::Dropped));
            };
            let future = writer.write(values.clone());
            match std::pin::pin!(future).poll(context) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(_)) => {
                    restore_owned_resources(&values, store.data_mut());
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
