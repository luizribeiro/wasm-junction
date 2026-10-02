use std::sync::{Arc, Mutex, MutexGuard};

use wasm_junction_core::{ImportTarget, Vals};
use wasmtime::component::{Linker, ResourceTable};
use wasmtime::{AsContextMut, StoreContextMut};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::engine::StoreData;

mod clocks;
mod gates;
mod linker;
mod trampoline;

pub(crate) use linker::add_ungated_interfaces;

pub(crate) struct WasiState {
    pub(crate) context: WasiCtx,
    pub(crate) table: ResourceTable,
}

impl WasiState {
    pub(crate) fn new(context: WasiCtx) -> Self {
        Self {
            context,
            table: ResourceTable::new(),
        }
    }
}

impl WasiView for WasiState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.context,
            table: &mut self.table,
        }
    }
}

pub(crate) fn add_gates(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gates::add_environment(linker)?;
    clocks::add_wall_clock_gate(linker)
}

pub(super) async fn dispatch(
    store: &mut StoreContextMut<'_, StoreData>,
    interface: &'static str,
    function: &'static str,
    args: Vals,
    target: Arc<dyn ImportTarget>,
) -> wasmtime::Result<Vals> {
    let (imports, context, component) = {
        let mut store = store.as_context_mut();
        let data = store.data_mut();
        (
            data.imports.clone(),
            data.context.clone(),
            data.component.clone(),
        )
    };
    imports
        .call_engine(
            context,
            component,
            Arc::from(interface),
            Arc::from(function),
            args,
            target,
        )
        .await
        .map_err(wasmtime::Error::new)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}
