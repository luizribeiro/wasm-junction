//! Builds Wasmtime-specific test components.

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=test-fixtures/wasi");
    wasm_junction_guest_build::build_wasi(
        "test-fixtures/wasi/Cargo.toml",
        "wasi_test_guest.wasm",
        "wasi-test.wasm",
    )?;
    println!("cargo::rerun-if-changed=test-fixtures/wasi-p3");
    wasm_junction_guest_build::build(
        "test-fixtures/wasi-p3/Cargo.toml",
        "wasi_p3_test_guest.wasm",
        "wasi-p3-test.wasm",
    )
}
