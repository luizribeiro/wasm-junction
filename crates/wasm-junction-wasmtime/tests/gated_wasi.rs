//! WASI interception coverage for the native engine.

#![cfg(feature = "wasi")]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use wasm_junction::{App, Call, CallError, Component, Middleware, Next, Vals};
use wasm_junction_wasmtime::{GATED_WASI_INTERFACES, WASI_INTERFACES};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const EXPORT: &str = "test:wasi/environment@0.1.0";

struct RecordGates(Arc<Mutex<BTreeSet<(String, String)>>>);

impl Middleware for RecordGates {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.starts_with("wasi:") {
            self.0
                .lock()
                .unwrap()
                .insert((call.interface.to_string(), call.function.to_string()));
        }
        next.run(call).await
    }
}

#[test]
fn gated_wasi_set_changes_only_deliberately() {
    assert_eq!(
        GATED_WASI_INTERFACES,
        [
            "wasi:cli/environment@0.2.12",
            "wasi:clocks/monotonic-clock@0.2.12",
            "wasi:clocks/wall-clock@0.2.12",
        ]
    );
}

#[test]
fn advertised_wasi_set_excludes_ungated_filesystem_and_sockets() {
    assert!(
        WASI_INTERFACES
            .iter()
            .all(|interface| !interface.starts_with("wasi:filesystem/")
                && !interface.starts_with("wasi:sockets/"))
    );
}

#[test]
fn every_function_in_each_gated_wit_interface_has_a_gate() {
    let seen = Arc::new(Mutex::new(BTreeSet::new()));
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(RecordGates(seen.clone()))
        .build()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("coverage")))
        .unwrap();
    runtime
        .block_on(app.call("coverage", EXPORT, "coverage", Vec::new()))
        .unwrap();

    assert_eq!(*seen.lock().unwrap(), wit_functions());
}

fn wit_functions() -> BTreeSet<(String, String)> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let resolved: Vec<_> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|package| package["name"] == "wasmtime-wasi")
        .collect();
    assert_eq!(resolved.len(), 1, "expected one resolved wasmtime-wasi");
    let manifest = resolved[0]["manifest_path"].as_str().unwrap();
    let wit = Path::new(manifest).parent().unwrap().join("src/p2/wit");
    let mut resolve = wit_parser::Resolve::default();
    resolve.push_dir(wit).unwrap();

    resolve
        .packages
        .iter()
        .flat_map(|(_, package)| {
            package.interfaces.iter().filter_map(|(name, interface)| {
                let name = package.name.interface_id(name);
                GATED_WASI_INTERFACES
                    .contains(&name.as_str())
                    .then_some((name, &resolve.interfaces[*interface]))
            })
        })
        .flat_map(|(interface_name, interface)| {
            interface
                .functions
                .keys()
                .map(move |function| (interface_name.clone(), function.clone()))
        })
        .collect()
}
