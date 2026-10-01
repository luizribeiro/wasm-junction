//! Builds the WebAssembly component used by the conformance fixture.

use wasm_junction_guest_build::build;

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=guest");
    println!("cargo::rerun-if-changed=wit");
    build(
        "guest/Cargo.toml",
        "notes_summary_guest.wasm",
        "notes-summary.wasm",
    )
}
