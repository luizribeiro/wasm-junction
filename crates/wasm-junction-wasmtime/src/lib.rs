//! A native wasm-junction engine powered by Wasmtime.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[allow(dead_code, reason = "shared conversion boundary for engine calls")]
mod values;
