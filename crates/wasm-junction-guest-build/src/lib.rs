//! Shared build-time support for guest components used by library crates.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
use std::{
    env::{var_os, vars_os},
    io,
    path::PathBuf,
    process::Command,
};
const TARGET: &str = "wasm32-unknown-unknown";
/// Builds a guest crate and componentizes its WebAssembly binary into `OUT_DIR`.
/// # Errors
/// Returns an error if `OUT_DIR` is absent or either command fails.
pub fn build(manifest: &str, wasm_binary: &str, output_name: &str) -> io::Result<()> {
    let output = PathBuf::from(var_os("OUT_DIR").ok_or(io::ErrorKind::NotFound)?);
    let target = output.join("guest-target");
    let mut cargo = Command::new(var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo.args(["build", "--release", "--locked", "--target", TARGET]);
    cargo
        .args(["--manifest-path", manifest, "--target-dir"])
        .arg(&target);
    for (key, _) in vars_os() {
        if key.to_string_lossy().starts_with("CARGO_")
            || key.to_string_lossy().starts_with("CLIPPY_")
            || key == "RUSTFLAGS"
        {
            cargo.env_remove(key);
        }
    }
    if !cargo.status()?.success() {
        return Err(io::ErrorKind::Other.into());
    }
    let mut component = Command::new("wasm-tools");
    component
        .args(["component", "new"])
        .arg(target.join(TARGET).join("release").join(wasm_binary))
        .arg("-o")
        .arg(output.join(output_name));
    component
        .status()?
        .success()
        .then_some(())
        .ok_or(io::ErrorKind::Other.into())
}
