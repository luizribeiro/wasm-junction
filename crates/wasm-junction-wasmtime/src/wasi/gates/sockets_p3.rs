#[allow(
    clippy::wildcard_imports,
    reason = "socket gates share the parent module's private gate machinery"
)]
use super::*;
use wasmtime_wasi::p3::bindings::sockets::types::TcpSocket;

const INTERFACE: &str = "wasi:sockets/types@0.3.0";
const TCP_SOCKET: &str = "tcp-socket";

#[allow(
    dead_code,
    unused_imports,
    unused_macros,
    reason = "the wrappers are consumed as socket families are registered"
)]
mod gate;
mod values;
