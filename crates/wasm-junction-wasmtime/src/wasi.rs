use std::collections::{HashMap, HashSet};

use wasmtime::component::{Linker, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
#[cfg(feature = "wasi-http")]
use wasmtime_wasi_http::WasiHttpCtx;

use crate::engine::StoreData;

mod gates;
mod trampoline;

pub(crate) struct WasiState {
    pub(crate) context: WasiCtx,
    pub(crate) table: ResourceTable,
    #[cfg(feature = "wasi-http")]
    pub(crate) http: WasiHttpCtx,
    channels: HashSet<u32>,
    descriptor_preopens: HashMap<u32, String>,
    #[allow(dead_code)]
    directory_stream_preopens: HashMap<u32, String>,
}

#[allow(dead_code)]
impl WasiState {
    pub(crate) fn new(context: WasiCtx) -> Self {
        Self {
            context,
            table: ResourceTable::new(),
            #[cfg(feature = "wasi-http")]
            http: WasiHttpCtx::new(),
            channels: HashSet::new(),
            descriptor_preopens: HashMap::new(),
            directory_stream_preopens: HashMap::new(),
        }
    }

    pub(crate) fn open_channel(&mut self, id: u32) -> bool {
        self.channels.insert(id)
    }

    pub(crate) fn close_channel(&mut self, id: u32) -> bool {
        self.channels.remove(&id)
    }

    pub(crate) fn set_descriptor_preopen(&mut self, id: u32, guest_path: String) {
        self.descriptor_preopens.insert(id, guest_path);
    }

    pub(crate) fn descriptor_preopen(&self, id: u32) -> Option<&str> {
        self.descriptor_preopens.get(&id).map(String::as_str)
    }

    pub(crate) fn remove_descriptor_preopen(&mut self, id: u32) {
        self.descriptor_preopens.remove(&id);
    }

    pub(crate) fn set_directory_stream_preopen(&mut self, id: u32, guest_path: String) {
        self.directory_stream_preopens.insert(id, guest_path);
    }

    pub(crate) fn directory_stream_preopen(&self, id: u32) -> Option<&str> {
        self.directory_stream_preopens.get(&id).map(String::as_str)
    }

    pub(crate) fn remove_directory_stream_preopen(&mut self, id: u32) {
        self.directory_stream_preopens.remove(&id);
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
    gates::add_exit(linker)?;
    gates::add_terminal(linker)?;
    gates::add_wall_clock(linker)?;
    gates::add_monotonic_clock(linker)?;
    gates::add_stdio(linker)?;
    gates::add_random(linker)?;
    gates::add_poll(linker)?;
    gates::add_streams(linker)?;
    gates::add_error(linker)?;
    gates::add_filesystem(linker)?;
    #[cfg(feature = "wasi-p3")]
    gates::add_p3(linker)?;
    #[cfg(feature = "wasi-http")]
    gates::add_http(linker)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use wasmtime_wasi::WasiCtxBuilder;

    use super::WasiState;

    #[test]
    fn channels_open_once_and_close_once() {
        let mut state = WasiState::new(WasiCtxBuilder::new().build());
        assert!(state.open_channel(7));
        assert!(!state.open_channel(7));
        assert!(state.close_channel(7));
        assert!(!state.close_channel(7));
    }

    #[test]
    #[cfg(feature = "wasi-p3")]
    fn concurrent_p3_gates_register() {
        crate::WasmtimeEngine::new().unwrap();
    }
}
