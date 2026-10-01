//! Builds the example's guest component.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET: &str = "wasm32-unknown-unknown";

fn main() {
    println!("cargo::rerun-if-changed=guest");
    println!("cargo::rerun-if-changed=wit");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let target = output.join("guest-target");
    let manifest = Path::new("guest/Cargo.toml");
    let mut cargo = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo
        .args(["build", "--release", "--locked", "--target", TARGET])
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--target-dir")
        .arg(&target);
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("CARGO_")
            || key.to_string_lossy().starts_with("CLIPPY_")
            || key == "RUSTFLAGS"
        {
            cargo.env_remove(key);
        }
    }
    run(&mut cargo, "guest build");
    run(
        Command::new("wasm-tools")
            .args(["component", "new"])
            .arg(target.join(TARGET).join("release/audit_guest.wasm"))
            .arg("-o")
            .arg(output.join("audit.wasm")),
        "component encoding",
    );
}

fn run(command: &mut Command, description: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("{description} could not start: {error}"));
    assert!(status.success(), "{description} failed with {status}");
}
