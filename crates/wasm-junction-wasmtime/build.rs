//! Builds Wasmtime-specific test components.

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=test-fixtures/wasi");
    wasm_junction_guest_build::build_wasi(
        "test-fixtures/wasi/Cargo.toml",
        "wasi_test_guest.wasm",
        "wasi-test.wasm",
    )?;
    println!("cargo::rerun-if-changed=test-fixtures/wasi-http-p2");
    wasm_junction_guest_build::build_wasi(
        "test-fixtures/wasi-http-p2/Cargo.toml",
        "wasi_http_p2_test_guest.wasm",
        "wasi-http-p2-test.wasm",
    )?;
    println!("cargo::rerun-if-changed=test-fixtures/wasi-p3");
    wasm_junction_guest_build::build(
        "test-fixtures/wasi-p3/Cargo.toml",
        "wasi_p3_test_guest.wasm",
        "wasi-p3-test.wasm",
    )?;
    println!("cargo::rerun-if-changed=test-fixtures/wasi-http");
    wasm_junction_guest_build::build(
        "test-fixtures/wasi-http/Cargo.toml",
        "wasi_http_test_guest.wasm",
        "wasi-http-test.wasm",
    )
}
