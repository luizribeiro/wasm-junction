//! Builds the WebAssembly component used by the conformance fixture.

use wasm_junction_guest_build::build;

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=guest");
    println!("cargo::rerun-if-changed=resource-wit");
    println!("cargo::rerun-if-changed=reload-wit");
    println!("cargo::rerun-if-changed=stream-wit");
    println!("cargo::rerun-if-changed=wit");
    build(
        "guest/Cargo.toml",
        "notes_summary_guest.wasm",
        "notes-summary.wasm",
    )?;
    build(
        "guest/Cargo.toml",
        "translator_guest.wasm",
        "translator.wasm",
    )?;
    build("guest/Cargo.toml", "writer_guest.wasm", "writer.wasm")?;
    build("guest/Cargo.toml", "cycle_a_guest.wasm", "cycle-a.wasm")?;
    build("guest/Cargo.toml", "cycle_b_guest.wasm", "cycle-b.wasm").and_then(|()| {
        build(
            "guest/Cargo.toml",
            "resource_client_guest.wasm",
            "resource-client.wasm",
        )
    })?;
    build("guest/Cargo.toml", "streams_guest.wasm", "streams.wasm")?;
    for (binary, output) in [
        ("reload_v1_guest.wasm", "reload-v1.wasm"),
        ("reload_v2_guest.wasm", "reload-v2.wasm"),
    ] {
        build("guest/Cargo.toml", binary, output)?;
    }
    Ok(())
}
