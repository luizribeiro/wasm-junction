//! Default engine dependency selection tests.

use std::process::Command;

const WASM_TARGETS: [&str; 2] = ["wasm32-unknown-unknown", "wasm32-wasip2"];

fn tree(target: &str, no_default_features: bool, features: Option<&str>) -> String {
    let manifest = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
    let mut command = Command::new(env!("CARGO"));
    command.args([
        "tree",
        "--manifest-path",
        &manifest,
        "--package",
        env!("CARGO_PKG_NAME"),
        "--target",
        target,
        "--edges",
        "normal",
        "--prefix",
        "none",
    ]);
    if no_default_features {
        command.arg("--no-default-features");
    }
    if let Some(features) = features {
        command.args(["--features", features]);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn contains_wasmtime(tree: &str) -> bool {
    tree.lines()
        .any(|line| line.split_whitespace().next() == Some("wasm-junction-wasmtime"))
}

fn contains_jco(tree: &str) -> bool {
    tree.lines()
        .any(|line| line.split_whitespace().next() == Some("wasm-junction-jco"))
}

#[test]
fn disabled_defaults_select_no_engine_on_any_target() {
    for target in [env!("WASM_JUNCTION_TARGET")]
        .into_iter()
        .chain(WASM_TARGETS)
    {
        assert!(!contains_wasmtime(&tree(target, true, None)), "{target}");
        assert!(!contains_jco(&tree(target, true, None)), "{target}");
    }
}

#[test]
fn defaults_select_each_engine_only_on_its_target() {
    assert!(contains_wasmtime(&tree(
        env!("WASM_JUNCTION_TARGET"),
        false,
        None
    )));
    assert!(!contains_jco(&tree(
        env!("WASM_JUNCTION_TARGET"),
        false,
        None
    )));
    assert!(contains_jco(&tree("wasm32-unknown-unknown", false, None)));
    assert!(!contains_wasmtime(&tree(
        "wasm32-unknown-unknown",
        false,
        None
    )));
    assert!(!contains_jco(&tree("wasm32-wasip2", false, None)));
    assert!(!contains_wasmtime(&tree("wasm32-wasip2", false, None)));
}

#[test]
fn wasi_dependency_follows_the_feature() {
    let without = tree(env!("WASM_JUNCTION_TARGET"), true, Some("wasmtime"));
    let with = tree(env!("WASM_JUNCTION_TARGET"), true, Some("wasmtime,wasi"));

    assert!(
        !without
            .lines()
            .any(|line| line.starts_with("wasmtime-wasi "))
    );
    assert!(with.lines().any(|line| line.starts_with("wasmtime-wasi ")));
}

#[test]
fn wasi_http_dependency_follows_the_feature() {
    let without = tree(env!("WASM_JUNCTION_TARGET"), true, Some("wasmtime,wasi-p3"));
    let with = tree(
        env!("WASM_JUNCTION_TARGET"),
        true,
        Some("wasmtime,wasi-http"),
    );

    assert!(
        !without
            .lines()
            .any(|line| line.starts_with("wasmtime-wasi-http "))
    );
    assert!(
        with.lines()
            .any(|line| line.starts_with("wasmtime-wasi-http "))
    );
}
