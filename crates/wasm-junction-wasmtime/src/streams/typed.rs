use wasm_junction_core::{ChannelDirection, StreamHandle};
use wasmtime::AsContextMut;
use wasmtime::component::StreamReader;

use crate::engine::StoreData;
use crate::stream_values::StreamValue;

mod consumer;
mod producer;

pub(crate) fn lift_static_stream<T: StreamValue>(
    stream: StreamReader<T>,
    mut store: impl AsContextMut<Data = StoreData>,
    direction: ChannelDirection,
) -> Result<StreamHandle, wasmtime::Error> {
    consumer::lift_reader(stream, store.as_context_mut(), direction)
}

pub(crate) fn lower_static_stream<T: StreamValue>(
    handle: StreamHandle,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamReader<T>, wasmtime::Error> {
    producer::lower_reader(handle, store.as_context_mut())
}
