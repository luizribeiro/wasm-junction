use wasmtime::component::ResourceTable;
use wasmtime_wasi::WasiCtx;

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
