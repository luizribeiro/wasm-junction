#[allow(
    clippy::wildcard_imports,
    reason = "socket gates share the parent module's private gate machinery"
)]
use super::*;
use wasmtime_wasi::p3::bindings::sockets::types::TcpSocket;

const INTERFACE: &str = "wasi:sockets/types@0.3.0";
const LOOKUP_INTERFACE: &str = "wasi:sockets/ip-name-lookup@0.3.0";
const TCP_SOCKET: &str = "tcp-socket";

#[allow(
    dead_code,
    unused_imports,
    unused_macros,
    reason = "the wrappers are consumed as socket families are registered"
)]
mod gate;
mod values;

fn sockets_enabled(store: &StoreData) -> bool {
    store
        .context
        .settings()
        .get::<wasm_junction_core::WasiSettings>()
        .is_some_and(wasm_junction_core::WasiSettings::sockets_enabled)
}

fn require_sockets(store: &StoreData) -> Result<(), CallError> {
    sockets_enabled(store)
        .then_some(())
        .ok_or_else(|| CallError::refused("sockets are disabled"))
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate::add_lookup(linker)
}
