//! Builds Wasmtime-specific test components.

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=test-fixtures/wasi");
    wasm_junction_guest_build::build_wasi(
        "test-fixtures/wasi/Cargo.toml",
        "wasi_test_guest.wasm",
        "wasi-test.wasm",
    )
}
