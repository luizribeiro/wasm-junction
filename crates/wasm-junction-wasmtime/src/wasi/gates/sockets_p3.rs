#[allow(
    clippy::wildcard_imports,
    reason = "socket gates share the parent module's private gate machinery"
)]
use super::*;
use wasmtime_wasi::p3::bindings::sockets::types::TcpSocket;

const INTERFACE: &str = "wasi:sockets/types@0.3.0";
const TCP_SOCKET: &str = "tcp-socket";

mod values;
