//! Builds the example's WASI guest component.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

const TARGET: &str = "wasm32-wasip2";

fn main() -> io::Result<()> {
    println!("cargo::rerun-if-changed=guest");
    println!("cargo::rerun-if-changed=wit");
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or(io::ErrorKind::NotFound)?);
    let target = output.join("guest-target");
    let mut cargo = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo.args(["build", "--release", "--locked", "--target", TARGET]);
    cargo
        .args(["--manifest-path", "guest/Cargo.toml", "--target-dir"])
        .arg(&target);
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("CARGO_")
            || key.to_string_lossy().starts_with("CLIPPY_")
            || key == "RUSTFLAGS"
        {
            cargo.env_remove(key);
        }
    }
    if !cargo.status()?.success() {
        return Err(io::Error::other("files guest build failed"));
    }
    fs::copy(
        target.join(TARGET).join("release/files_guest.wasm"),
        output.join("files.wasm"),
    )?;
    Ok(())
}
