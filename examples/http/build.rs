//! Builds the example's WebAssembly component.

#![forbid(unsafe_code)]

use std::env;
use std::io;
use std::path::PathBuf;
use std::process::Command;

const TARGET: &str = "wasm32-unknown-unknown";

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
        return Err(io::Error::other("HTTP guest build failed"));
    }
    let module = target.join(TARGET).join("release/http_guest.wasm");
    let status = Command::new("wasm-tools")
        .args(["component", "new"])
        .arg(module)
        .arg("-o")
        .arg(output.join("http.wasm"))
        .status()?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| io::Error::other("HTTP guest componentization failed"))
}
