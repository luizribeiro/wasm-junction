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

/// WASI Preview 2 interfaces intercepted by the application middleware chain.
pub const GATED_WASI_INTERFACES: &[&str] = &[
    "wasi:cli/environment@0.2.12",
    "wasi:clocks/monotonic-clock@0.2.12",
    "wasi:clocks/wall-clock@0.2.12",
];
