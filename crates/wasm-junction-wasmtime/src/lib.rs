//! A native wasm-junction engine powered by Wasmtime.
//!
//! All WASI Preview 2 interfaces are available. Calls to [`GATED_WASI_INTERFACES`] are
//! intercepted by application middleware; the remaining WASI interfaces use wasmtime-wasi
//! directly.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod engine;
mod imports;
mod values;
mod wasi;

pub use engine::WasmtimeEngine;

/// WASI Preview 2 interfaces intercepted by the application middleware chain.
pub const GATED_WASI_INTERFACES: &[&str] = &[
    "wasi:cli/environment@0.2.12",
    "wasi:clocks/monotonic-clock@0.2.12",
    "wasi:clocks/wall-clock@0.2.12",
];
