use wasm_junction_core::{ChannelDirection, StreamHandle};
use wasmtime::component::{StreamAny, StreamReader, Type};
use wasmtime::{AsContextMut, StoreContextMut};

use crate::engine::StoreData;
use crate::stream_types::{StreamTypeVisitor, visit_stream_type};
use crate::stream_values::StreamValue;

use super::{lift_stream_with_direction, lower_stream};

mod consumer;
mod producer;

pub(crate) fn lift_stream(
    stream: StreamAny,
    item_type: Option<Type>,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamHandle, wasmtime::Error> {
    let item_type = item_type.ok_or_else(|| wasmtime::Error::msg("stream has no item type"))?;
    if item_type == Type::U8 {
        return lift_stream_with_direction(stream, store, ChannelDirection::GuestToHost);
    }
    match bridge(
        &item_type.clone(),
        Bridge {
            operation: stream.into(),
            item_type,
            store: store.as_context_mut(),
        },
    )? {
        Output::Handle(handle) => Ok(handle),
        Output::Runtime(_) => Err(wasmtime::Error::msg("invalid typed stream lift")),
    }
}

pub(crate) fn lower_typed_stream(
    handle: StreamHandle,
    item_type: Type,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamAny, wasmtime::Error> {
    if item_type == Type::U8 {
        return lower_stream(handle, store);
    }
    match bridge(
        &item_type.clone(),
        Bridge {
            operation: handle.into(),
            item_type,
            store: store.as_context_mut(),
        },
    )? {
        Output::Runtime(stream) => Ok(stream),
        Output::Handle(_) => Err(wasmtime::Error::msg("invalid typed stream lowering")),
    }
}

#[cfg(feature = "wasi-p3")]
pub(crate) fn lift_static_stream<T: StreamValue>(
    stream: StreamReader<T>,
    mut store: impl AsContextMut<Data = StoreData>,
    direction: ChannelDirection,
) -> Result<StreamHandle, wasmtime::Error> {
    consumer::lift_reader(stream, None, store.as_context_mut(), direction)
}

pub(crate) fn recover_exported_stream(
    stream: StreamAny,
    item_type: Option<Type>,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<StreamHandle, wasmtime::Error> {
    let item_type = item_type.ok_or_else(|| wasmtime::Error::msg("stream has no item type"))?;
    if item_type == Type::U8 {
        return recover_reader(
            stream.try_into_stream_reader::<u8>()?,
            store.as_context_mut(),
        );
    }
    match bridge(
        &item_type.clone(),
        Bridge {
            operation: Operation::Recover(stream),
            item_type,
            store: store.as_context_mut(),
        },
    )? {
        Output::Handle(handle) => Ok(handle),
        Output::Runtime(_) => Err(wasmtime::Error::msg("invalid stream recovery")),
    }
}

fn bridge<V: StreamTypeVisitor>(
    item_type: &Type,
    visitor: V,
) -> Result<V::Output, wasmtime::Error> {
    visit_stream_type(item_type, visitor)
}

enum Operation {
    Lift(StreamAny),
    Lower(StreamHandle),
    Recover(StreamAny),
}

impl From<StreamAny> for Operation {
    fn from(stream: StreamAny) -> Self {
        Self::Lift(stream)
    }
}

impl From<StreamHandle> for Operation {
    fn from(stream: StreamHandle) -> Self {
        Self::Lower(stream)
    }
}

enum Output {
    Handle(StreamHandle),
    Runtime(StreamAny),
}

struct Bridge<'a> {
    operation: Operation,
    item_type: Type,
    store: StoreContextMut<'a, StoreData>,
}

impl StreamTypeVisitor for Bridge<'_> {
    type Output = Output;

    fn visit<T: StreamValue>(self) -> Result<Self::Output, wasmtime::Error> {
        match self.operation {
            Operation::Lift(stream) => {
                consumer::lift::<T>(stream, self.item_type, self.store).map(Output::Handle)
            }
            Operation::Lower(handle) => {
                producer::lower::<T>(handle, self.item_type, self.store).map(Output::Runtime)
            }
            Operation::Recover(stream) => {
                recover_reader(stream.try_into_stream_reader::<T>()?, self.store)
                    .map(Output::Handle)
            }
        }
    }
}

fn recover_reader<T: 'static>(
    reader: StreamReader<T>,
    mut store: StoreContextMut<'_, StoreData>,
) -> Result<StreamHandle, wasmtime::Error> {
    match reader.try_into::<StreamHandle>(store.as_context_mut()) {
        Ok(handle) => Ok(handle),
        Err(mut reader) => {
            reader.close(store.as_context_mut())?;
            Err(wasmtime::Error::new(
                wasm_junction_core::CallError::refused(
                    "guest-created streams cannot be returned because the Wasmtime store ends with each call",
                ),
            ))
        }
    }
}
