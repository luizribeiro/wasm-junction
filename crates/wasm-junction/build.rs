//! Makes Cargo's exact build target available to runtime diagnostics.

fn main() {
    if let Ok(target) = std::env::var("TARGET") {
        println!("cargo:rustc-env=WASM_JUNCTION_TARGET={target}");
    }
}
