//! Shared build-time support for guest components used by library crates.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
use std::{
    env::{var_os, vars_os},
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};
const TARGET: &str = "wasm32-unknown-unknown";
const WASI_TARGET: &str = "wasm32-wasip2";
/// Builds a guest crate and componentizes its WebAssembly binary into `OUT_DIR`.
/// # Errors
/// Returns an error if `OUT_DIR` is absent or either command fails.
pub fn build(manifest: &str, wasm_binary: &str, output_name: &str) -> io::Result<()> {
    build_target(manifest, wasm_binary, output_name, TARGET, true)
}

/// Builds a WASI Preview 2 guest crate into `OUT_DIR`.
///
/// # Errors
/// Returns an error if `OUT_DIR` is absent or the guest build or copy fails.
pub fn build_wasi(manifest: &str, wasm_binary: &str, output_name: &str) -> io::Result<()> {
    build_target(manifest, wasm_binary, output_name, WASI_TARGET, false)
}

fn build_target(
    manifest: &str,
    wasm_binary: &str,
    output_name: &str,
    target: &str,
    componentize: bool,
) -> io::Result<()> {
    let output = PathBuf::from(var_os("OUT_DIR").ok_or(io::ErrorKind::NotFound)?);
    let target_dir = guest_target_dir(&output, Path::new(manifest));
    let mut cargo = Command::new(var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo.args(["build", "--release", "--locked", "--target", target]);
    cargo
        .args(["--manifest-path", manifest, "--target-dir"])
        .arg(&target_dir);
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
    let artifact = target_dir.join(target).join("release").join(wasm_binary);
    if !componentize {
        return fs::copy(artifact, output.join(output_name)).map(|_| ());
    }
    let mut component = Command::new("wasm-tools");
    component
        .args(["component", "new"])
        .arg(artifact)
        .arg("-o")
        .arg(output.join(output_name));
    component
        .status()?
        .success()
        .then_some(())
        .ok_or(io::ErrorKind::Other.into())
}

/// Every feature combination and check/build of a host gets its own `OUT_DIR`, but the guest
/// build doesn't depend on any of them, so guests share one directory per manifest under the
/// host's target directory, which Cargo marks with `CACHEDIR.TAG`.
fn guest_target_dir(out_dir: &Path, manifest: &Path) -> PathBuf {
    let Some(target) = out_dir
        .ancestors()
        .find(|dir| dir.join("CACHEDIR.TAG").is_file())
    else {
        return out_dir.join("guest-target");
    };
    let manifest = fs::canonicalize(manifest).unwrap_or_else(|_| manifest.to_path_buf());
    let name: String = manifest
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    target.join("guests").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guests_share_a_directory_under_the_target_dir() {
        let target = std::env::temp_dir().join(format!("guest-build-{}", std::process::id()));
        let out_p3 = target.join("debug/build/host-1111/out");
        let out_http = target.join("debug/build/host-2222/out");
        fs::create_dir_all(&out_p3).unwrap();
        fs::create_dir_all(&out_http).unwrap();
        fs::write(target.join("CACHEDIR.TAG"), "").unwrap();
        let manifest = Path::new("tests/fixtures/guest/Cargo.toml");

        let shared = guest_target_dir(&out_p3, manifest);

        assert_eq!(shared, guest_target_dir(&out_http, manifest));
        assert!(shared.starts_with(target.join("guests")));
        assert_ne!(
            shared,
            guest_target_dir(&out_p3, Path::new("other/Cargo.toml"))
        );
        fs::remove_dir_all(&target).unwrap();
    }

    #[test]
    fn without_a_target_dir_the_guest_builds_in_out_dir() {
        let out = Path::new("/nonexistent/build/host/out");

        assert_eq!(
            guest_target_dir(out, Path::new("guest/Cargo.toml")),
            out.join("guest-target")
        );
    }
}
