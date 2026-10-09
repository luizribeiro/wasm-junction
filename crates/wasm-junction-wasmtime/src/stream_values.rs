use wasm_junction_core::{CallError, Val};
use wasmtime::AsContextMut;
use wasmtime::component::{ComponentType, Lift, Lower, ResourceAny, Type};

use crate::engine::{StoreData, lift_resource, lower_resource};
use crate::values::expected_resource;

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

macro_rules! scalar {
    ($ty:ty, $variant:ident) => {
        impl StreamValue for $ty {
            fn into_val(
                self,
                _ty: Option<&Type>,
                _store: &mut wasmtime::StoreContextMut<'_, StoreData>,
            ) -> Result<Option<Val>, wasmtime::Error> {
                Ok(Some(Val::$variant(self)))
            }

            fn from_val(
                value: Option<Val>,
                _ty: Option<&Type>,
                _store: &mut wasmtime::StoreContextMut<'_, StoreData>,
            ) -> Result<Self, wasmtime::Error> {
                match value {
                    Some(Val::$variant(value)) => Ok(value),
                    value => Err(shape(stringify!($ty), value.as_ref())),
                }
            }
        }
    };
}

scalar!(bool, Bool);
scalar!(i8, S8);
scalar!(u8, U8);
scalar!(i16, S16);
scalar!(u16, U16);
scalar!(i32, S32);
scalar!(u32, U32);
scalar!(i64, S64);
scalar!(u64, U64);
scalar!(f32, F32);
scalar!(f64, F64);
scalar!(char, Char);
scalar!(String, String);

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

impl StreamValue for ResourceAny {
    fn into_val(
        self,
        _ty: Option<&Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Option<Val>, wasmtime::Error> {
        lift_resource(self, store.as_context_mut())
            .map(Val::Resource)
            .map(Some)
    }

    fn from_val(
        value: Option<Val>,
        ty: Option<&Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Self, wasmtime::Error> {
        match value {
            Some(Val::Resource(value)) => {
                lower_resource(&value, expected_resource(ty), store.as_context_mut())
            }
            value => Err(shape("resource", value.as_ref())),
        }
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
    #[test]
    fn scalar_items_convert_in_both_directions() {
        let mut store = test_store();
        let mut context = store.as_context_mut();
        assert_eq!(
            42_u32.into_val(None, &mut context).unwrap(),
            Some(Val::U32(42))
        );
        assert_eq!(
            String::from_val(Some(Val::from("notes")), None, &mut context).unwrap(),
            "notes"
        );
        assert_eq!(
            bool::from_val(Some(Val::String("wrong".to_owned())), None, &mut context)
                .unwrap_err()
                .to_string(),
            "expected bool stream item, got Some(String(\"wrong\"))"
        );
    }
}
