//! Builds the example's WebAssembly components.

#![forbid(unsafe_code)]

use std::env;
use std::io;
use std::path::PathBuf;
use std::process::Command;

const TARGET: &str = "wasm32-unknown-unknown";
const COMPONENTS: [(&str, &str); 3] = [
    ("agent_guest.wasm", "agent.wasm"),
    ("notes_tools_guest.wasm", "notes-tools.wasm"),
    ("web_tools_guest.wasm", "web-tools.wasm"),
];

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
        return Err(io::Error::other("example guest build failed"));
    }
    for (module, component) in COMPONENTS {
        let status = Command::new("wasm-tools")
            .args(["component", "new"])
            .arg(target.join(TARGET).join("release").join(module))
            .arg("-o")
            .arg(output.join(component))
            .status()?;
        if !status.success() {
            return Err(io::Error::other("example componentization failed"));
        }
    }
    Ok(())
}
