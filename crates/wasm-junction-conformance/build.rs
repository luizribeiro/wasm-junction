//! Builds the WebAssembly component used by the conformance fixture.

use std::env::var_os;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use wasm_junction_guest_build::build;

fn main() -> io::Result<()> {
    println!("cargo::rerun-if-changed=guest");
    println!("cargo::rerun-if-changed=dispatch-wit");
    println!("cargo::rerun-if-changed=resource-wit");
    println!("cargo::rerun-if-changed=resource-stream-wit");
    println!("cargo::rerun-if-changed=reload-wit");
    println!("cargo::rerun-if-changed=stream-wit");
    println!("cargo::rerun-if-changed=value-stream-wit");
    println!("cargo::rerun-if-changed=named-value-stream-wit");
    println!("cargo::rerun-if-changed=wit");
    println!("cargo::rerun-if-changed=trap-component.wat");
    parse_trap_component()?;
    build(
        "guest/Cargo.toml",
        "notes_summary_guest.wasm",
        "notes-summary.wasm",
    )?;
    build("guest/Cargo.toml", "dispatch_guest.wasm", "dispatch.wasm")?;
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
    build(
        "guest/Cargo.toml",
        "value_streams_guest.wasm",
        "value-streams.wasm",
    )?;
    build(
        "guest/Cargo.toml",
        "named_value_streams_guest.wasm",
        "named-value-streams.wasm",
    )?;
    build(
        "guest/Cargo.toml",
        "resource_stream_guest.wasm",
        "resource-stream.wasm",
    )?;
    for (binary, output) in [
        ("reload_v1_guest.wasm", "reload-v1.wasm"),
        ("reload_v2_guest.wasm", "reload-v2.wasm"),
        ("reload_breaking_guest.wasm", "reload-breaking.wasm"),
        ("reload_writer_guest.wasm", "reload-writer.wasm"),
    ] {
        build("guest/Cargo.toml", binary, output)?;
    }
    Ok(())
}

fn parse_trap_component() -> io::Result<()> {
    let output = PathBuf::from(var_os("OUT_DIR").ok_or(io::ErrorKind::NotFound)?)
        .join("trap-component.wasm");
    Command::new("wasm-tools")
        .args(["parse", "trap-component.wat", "-o"])
        .arg(output)
        .status()?
        .success()
        .then_some(())
        .ok_or(io::ErrorKind::Other.into())
}
