//! A native wasm-junction engine powered by Wasmtime.
//!
//! All WASI Preview 2 interfaces are available. Calls to [`GATED_WASI_INTERFACES`] are
//! intercepted by application middleware; the remaining WASI interfaces use wasmtime-wasi
//! directly.
//!
//! The default `parallel-compilation` feature lets Wasmtime compile functions across every
//! available core. Disable default features when predictable CPU use matters more than load and
//! reload latency. Calls made during compilation can take longer while the cores are occupied;
//! in the reload benchmark their median latency rose from about 27 us to about 50 us.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod engine;
mod imports;
mod streams;
mod values;
mod wasi;

pub use engine::WasmtimeEngine;

/// WASI Preview 2 interfaces supplied by [`WasmtimeEngine`].
pub const WASI_INTERFACES: &[&str] = &[
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

/// WASI Preview 2 interfaces intercepted by the application middleware chain.
pub const GATED_WASI_INTERFACES: &[&str] = &[
    "wasi:cli/environment@0.2.12",
    "wasi:clocks/monotonic-clock@0.2.12",
    "wasi:clocks/wall-clock@0.2.12",
];
