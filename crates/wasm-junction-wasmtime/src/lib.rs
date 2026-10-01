//! A native wasm-junction engine powered by Wasmtime.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod engine;
mod imports;
mod values;

pub use engine::WasmtimeEngine;
