use wasmtime::component::{Linker, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::engine::StoreData;

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
    gates::add_wall_clock(linker)?;
    gates::add_monotonic_clock(linker)
}
