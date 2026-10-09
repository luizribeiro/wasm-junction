use wasm_junction_core::StreamHandle;
use wasmtime::component::{StreamAny, StreamReader, Type};
use wasmtime::{AsContextMut, StoreContextMut};

use crate::engine::StoreData;
use crate::stream_types::{StreamTypeVisitor, visit_stream_type};

mod consumer;

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
