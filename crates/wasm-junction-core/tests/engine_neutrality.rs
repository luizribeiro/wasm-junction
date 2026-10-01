//! Core dependency boundary tests.

use std::process::Command;

#[test]
fn core_dependency_tree_contains_no_engine_or_browser_runtime() {
    let manifest = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--manifest-path",
            &manifest,
            "--package",
            env!("CARGO_PKG_NAME"),
            "--edges",
            "normal",
            "--all-features",
            "--prefix",
            "none",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let tree = String::from_utf8(output.stdout).unwrap();
    let forbidden = ["wasmtime", "wasm-bindgen", "js-sys", "web-sys"];
    for line in tree.lines() {
        let package = line.split_whitespace().next().unwrap_or_default();
        assert!(
            !forbidden.contains(&package),
            "core dependency tree contains forbidden package `{package}`:\n{tree}"
        );
    }
}
