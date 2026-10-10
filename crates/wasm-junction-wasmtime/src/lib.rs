//! A native wasm-junction engine powered by Wasmtime.
//!
//! With the default `wasi` feature, WASI Preview 2 interfaces are available through the explicit
//! provider. The opt-in `wasi-p3` feature adds the supported Preview 3 interfaces. Every supplied
//! WASI call is intercepted by application middleware.
//!
//! The default `parallel-compilation` feature lets Wasmtime compile functions across every
//! available core. Disable default features when predictable CPU use matters more than load and
//! reload latency. Calls made during compilation can take longer while the cores are occupied;
//! in the reload benchmark their median latency rose from about 27 us to about 50 us.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod engine;
mod futures;
mod imports;
mod stream_types;
mod stream_values;
mod streams;
mod values;
#[cfg(feature = "wasi")]
mod wasi;

#[cfg(feature = "wasi")]
const STATIC_STREAM_INTERFACES: &[&str] = wasi::STATIC_STREAM_INTERFACES;
#[cfg(not(feature = "wasi"))]
const STATIC_STREAM_INTERFACES: &[&str] = &[];

pub use engine::WasmtimeEngine;

#[cfg(feature = "wasi")]
const P2_WASI_INTERFACES: &[&str] = &[
    "wasi:cli/environment@0.2.12",
    "wasi:cli/exit@0.2.12",
    "wasi:cli/stderr@0.2.12",
    "wasi:cli/stdin@0.2.12",
    "wasi:cli/stdout@0.2.12",
    "wasi:cli/terminal-input@0.2.12",
    "wasi:cli/terminal-output@0.2.12",
    "wasi:cli/terminal-stderr@0.2.12",
    "wasi:cli/terminal-stdin@0.2.12",
    "wasi:cli/terminal-stdout@0.2.12",
    "wasi:clocks/monotonic-clock@0.2.12",
    "wasi:clocks/wall-clock@0.2.12",
    "wasi:filesystem/preopens@0.2.12",
    "wasi:filesystem/types@0.2.12",
    "wasi:io/error@0.2.12",
    "wasi:io/poll@0.2.12",
    "wasi:io/streams@0.2.12",
    "wasi:random/insecure-seed@0.2.12",
    "wasi:random/insecure@0.2.12",
    "wasi:random/random@0.2.12",
    "wasi:sockets/instance-network@0.2.12",
    "wasi:sockets/ip-name-lookup@0.2.12",
    "wasi:sockets/network@0.2.12",
    "wasi:sockets/tcp-create-socket@0.2.12",
    "wasi:sockets/tcp@0.2.12",
    "wasi:sockets/udp-create-socket@0.2.12",
    "wasi:sockets/udp@0.2.12",
];

#[cfg(feature = "wasi-p3")]
const P3_WASI_INTERFACES: [&str; 17] = [
    "wasi:cli/environment@0.3.0",
    "wasi:cli/exit@0.3.0",
    "wasi:cli/types@0.3.0",
    "wasi:cli/stderr@0.3.0",
    "wasi:cli/stdin@0.3.0",
    "wasi:cli/stdout@0.3.0",
    "wasi:cli/terminal-input@0.3.0",
    "wasi:cli/terminal-output@0.3.0",
    "wasi:cli/terminal-stderr@0.3.0",
    "wasi:cli/terminal-stdin@0.3.0",
    "wasi:cli/terminal-stdout@0.3.0",
    "wasi:clocks/types@0.3.0",
    "wasi:clocks/monotonic-clock@0.3.0",
    "wasi:clocks/system-clock@0.3.0",
    "wasi:random/insecure-seed@0.3.0",
    "wasi:random/insecure@0.3.0",
    "wasi:random/random@0.3.0",
];

/// WASI interfaces supplied by [`WasmtimeEngine`].
#[cfg(all(feature = "wasi", not(feature = "wasi-p3")))]
pub const WASI_INTERFACES: &[&str] = P2_WASI_INTERFACES;

/// WASI interfaces supplied by [`WasmtimeEngine`].
#[cfg(feature = "wasi-p3")]
pub const WASI_INTERFACES: &[&str] = &{
    let mut interfaces = [""; P2_WASI_INTERFACES.len() + P3_WASI_INTERFACES.len()];
    let mut index = 0;
    while index < P2_WASI_INTERFACES.len() {
        interfaces[index] = P2_WASI_INTERFACES[index];
        index += 1;
    }
    let mut p3 = 0;
    while p3 < P3_WASI_INTERFACES.len() {
        interfaces[index] = P3_WASI_INTERFACES[p3];
        index += 1;
        p3 += 1;
    }
    interfaces
};

/// WASI interfaces supplied by [`WasmtimeEngine`].
///
/// Every interface supplied by the provider is gated.
#[cfg(feature = "wasi")]
pub const GATED_WASI_INTERFACES: &[&str] = WASI_INTERFACES;

/// WASI outgoing HTTP interfaces supplied by [`WasmtimeEngine`].
#[cfg(all(feature = "wasi-http", not(feature = "wasi-p3")))]
pub const WASI_HTTP_INTERFACES: &[&str] = &[
    "wasi:http/outgoing-handler@0.2.12",
    "wasi:http/types@0.2.12",
];

/// WASI outgoing HTTP interfaces supplied by [`WasmtimeEngine`].
#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
pub const WASI_HTTP_INTERFACES: &[&str] = &[
    "wasi:http/outgoing-handler@0.2.12",
    "wasi:http/types@0.2.12",
    "wasi:http/client@0.3.0",
    "wasi:http/types@0.3.0",
];
