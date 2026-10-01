//! Builds the guest used by the dispatch benchmark.

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=benchmark");
    wasm_junction_guest_build::build(
        "benchmark/guest/Cargo.toml",
        "dispatch_benchmark_guest.wasm",
        "dispatch-benchmark.wasm",
    )
}
