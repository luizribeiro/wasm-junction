use wasm_junction_core::Val;
use wasmtime::component::{ComponentType, Lift, Lower};

use crate::engine::StoreData;

pub(crate) trait StreamValue:
    ComponentType + Lift + Lower + Send + Sync + Unpin + Sized + 'static
{
    fn into_val(
        self,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Val, wasmtime::Error>;

    fn from_val(
        value: Val,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Self, wasmtime::Error>;
}
