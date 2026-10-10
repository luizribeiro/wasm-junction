#[allow(
    clippy::wildcard_imports,
    reason = "socket gates share the parent module's private gate machinery"
)]
use super::*;
use wasmtime_wasi::p3::bindings::sockets::types::{TcpSocket, UdpSocket};

pub(super) const INTERFACE: &str = "wasi:sockets/types@0.3.0";
const LOOKUP_INTERFACE: &str = "wasi:sockets/ip-name-lookup@0.3.0";
const TCP_SOCKET: &str = "tcp-socket";
const UDP_SOCKET: &str = "udp-socket";

mod gate;
mod tcp;
#[cfg(test)]
mod test_support;
mod udp;
mod values;

impl WitResource for TcpSocket {
    const INTERFACE: &'static str = INTERFACE;
    const NAME: &'static str = TCP_SOCKET;
}

impl WitResource for UdpSocket {
    const INTERFACE: &'static str = INTERFACE;
    const NAME: &'static str = UDP_SOCKET;
}

fn validate_tcp(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_borrowed::<TcpSocket>(values.first().ok_or_else(|| shape(TCP_SOCKET))?, store)?;
    validate_handle_contexts(values, store)
}

fn validate_udp(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_borrowed::<UdpSocket>(values.first().ok_or_else(|| shape(UDP_SOCKET))?, store)?;
    validate_handle_contexts(values, store)
}

fn validate_none(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_handle_contexts(values, store)
}

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
    tcp::add(linker)?;
    udp::add(linker)?;
    gate::add_lookup(linker)
}
