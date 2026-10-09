use wasm_junction_core::{CallError, Val};
use wasmtime::component::{ComponentType, Lift, Lower, Type};

use crate::engine::StoreData;

pub(crate) trait StreamValue:
    ComponentType + Lift + Lower + Send + Sync + Unpin + Sized + 'static
{
    fn into_val(
        self,
        ty: Option<&Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Option<Val>, wasmtime::Error>;

    fn from_val(
        value: Option<Val>,
        ty: Option<&Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Self, wasmtime::Error>;
}

impl StreamValue for () {
    fn into_val(
        self,
        _ty: Option<&Type>,
        _store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Option<Val>, wasmtime::Error> {
        Ok(None)
    }

    fn from_val(
        value: Option<Val>,
        _ty: Option<&Type>,
        _store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Self, wasmtime::Error> {
        value
            .is_none()
            .then_some(())
            .ok_or_else(|| shape("unit", value.as_ref()))
    }
}

pub(super) fn shape(expected: &str, value: Option<&Val>) -> wasmtime::Error {
    wasmtime::Error::new(CallError::refused(format!(
        "expected {expected} stream item, got {value:?}"
    )))
}
#[cfg(test)]
struct TestImports;

#[cfg(test)]
impl wasm_junction_core::ImportDispatcher for TestImports {
    fn call(
        &self,
        _context: wasm_junction_core::InvocationContext,
        _caller: std::sync::Arc<str>,
        _interface: std::sync::Arc<str>,
        _function: std::sync::Arc<str>,
        _args: Vec<Val>,
    ) -> wasm_junction_core::BoxFuture<'_, Result<Vec<Val>, CallError>> {
        Box::pin(async { Err(CallError::trap("unexpected imported call")) })
    }

    fn call_engine(
        &self,
        _context: wasm_junction_core::InvocationContext,
        _caller: std::sync::Arc<str>,
        _interface: std::sync::Arc<str>,
        _function: std::sync::Arc<str>,
        _args: Vec<Val>,
        _target: std::sync::Arc<dyn wasm_junction_core::ImportTarget>,
    ) -> wasm_junction_core::BoxFuture<'_, Result<Vec<Val>, CallError>> {
        Box::pin(async { Err(CallError::trap("unexpected engine call")) })
    }

    fn drop_resource(
        &self,
        _context: wasm_junction_core::InvocationContext,
        _caller: std::sync::Arc<str>,
        _resource: wasm_junction_core::Resource,
    ) -> wasm_junction_core::BoxFuture<'_, Result<(), CallError>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
pub(crate) fn test_store() -> wasmtime::Store<StoreData> {
    let data = StoreData::for_value_test(std::sync::Arc::new(TestImports)).unwrap();
    wasmtime::Store::new(&wasmtime::Engine::default(), data)
}

#[cfg(test)]
mod tests {
    use wasmtime::AsContextMut;

    use super::*;

    #[test]
    fn unit_items_round_trip_without_a_value() {
        let mut store = test_store();
        let mut context = store.as_context_mut();
        let value = ().into_val(None, &mut context).unwrap();
        assert_eq!(value, None);
        assert_eq!(<()>::from_val(value, None, &mut context).unwrap(), ());
    }
}
