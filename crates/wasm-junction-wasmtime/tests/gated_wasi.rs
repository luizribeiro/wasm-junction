//! WASI interception coverage for the native engine.

#![cfg(feature = "wasi")]

use wasm_junction_wasmtime::GATED_WASI_INTERFACES;

#[test]
fn gated_wasi_set_changes_only_deliberately() {
    assert_eq!(
        GATED_WASI_INTERFACES,
        [
            "wasi:cli/environment@0.2.12",
            "wasi:clocks/monotonic-clock@0.2.12",
            "wasi:clocks/wall-clock@0.2.12",
        ]
    );
}
