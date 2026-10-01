//! Builds the guest used by the dispatch benchmark.

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=benchmark");
    wasm_junction_guest_build::build(
        "benchmark/guest/Cargo.toml",
        "dispatch_benchmark_guest.wasm",
        "dispatch-benchmark.wasm",
    )?;
    println!("cargo::rerun-if-changed=test-fixtures/wasi");
    wasm_junction_guest_build::build_wasi(
        "test-fixtures/wasi/Cargo.toml",
        "wasi_test_guest.wasm",
        "wasi-test.wasm",
    )
}
