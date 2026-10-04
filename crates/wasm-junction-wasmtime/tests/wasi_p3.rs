//! WASI Preview 3 feature and gate behavior.

#![cfg(feature = "wasi")]

use std::future::Future;
#[cfg(feature = "wasi-p3")]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "wasi-p3")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "wasi-p3")]
use std::{collections::BTreeSet, path::Path, process::Command};

use wasm_junction::LoadError;
use wasm_junction::{App, Component};
#[cfg(feature = "wasi-p3")]
use wasm_junction::{Call, CallError, CallErrorKind, Middleware, Next, Val, Vals};
#[cfg(feature = "wasi-p3")]
use wasm_junction_wasmtime::WASI_INTERFACES;
use wasm_junction_wasmtime::WasmtimeEngine;
#[cfg(feature = "wasi-p3")]
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
#[cfg(feature = "wasi-p3")]
use wit_parser::{ManglingAndAbi, Resolve};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-p3-test.wasm"));
#[cfg(feature = "wasi-p3")]
const EXPORT: &str = "test:wasi-p3/probe@0.1.0";

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
#[cfg(not(feature = "wasi-p3"))]
fn p3_imports_are_missing_when_the_feature_is_disabled() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    let component = Component::from_bytes(COMPONENT).unwrap().named("p3");
    let mut expected = component.imports().to_vec();
    expected.sort();

    let LoadError::MissingImports(missing) = block_on(app.load(component)).unwrap_err() else {
        panic!("expected missing p3 imports");
    };
    assert_eq!(missing.interfaces(), expected);
}

#[cfg(feature = "wasi-p3")]
struct WaitBehavior {
    refuse: bool,
    calls: Arc<AtomicUsize>,
}

#[cfg(feature = "wasi-p3")]
impl Middleware for WaitBehavior {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        let wait = call.interface.as_ref() == "wasi:clocks/monotonic-clock@0.3.0"
            && matches!(call.function.as_ref(), "wait-for" | "wait-until");
        if wait {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.refuse && call.function.as_ref() == "wait-for" {
                return Err(CallError::refused("p3 wait denied"));
            }
            call.args = vec![Val::U64(0)];
        }
        next.run(call).await
    }
}

#[cfg(feature = "wasi-p3")]
fn p3_app(middleware: impl Middleware + 'static) -> App {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(middleware)
        .build()
        .unwrap();
    app.configure(
        "p3",
        wasm_junction::WasiSettings::new()
            .env("GREETING", "hello")
            .arg("alpha"),
    )
    .unwrap();
    block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("p3"))).unwrap();
    app
}

#[test]
#[cfg(feature = "wasi-p3")]
fn p3_clock_waits_can_await_middleware_and_be_rewritten() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = p3_app(WaitBehavior {
        refuse: false,
        calls: calls.clone(),
    });
    let values = block_on(app.call("p3", EXPORT, "coverage", Vec::new())).unwrap();
    assert_eq!(values, [Val::from("true|true|4|5|true|true")]);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
#[cfg(feature = "wasi-p3")]
fn refusal_of_a_p3_function_without_an_error_result_traps() {
    let app = p3_app(WaitBehavior {
        refuse: true,
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let error = block_on(app.call("p3", EXPORT, "coverage", Vec::new())).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "p3 wait denied");
}

#[cfg(feature = "wasi-p3")]
struct RecordGates(Arc<Mutex<BTreeSet<(String, String)>>>);

#[cfg(feature = "wasi-p3")]
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
#[cfg(feature = "wasi-p3")]
fn every_function_in_each_gated_p3_interface_has_a_gate() {
    let seen = Arc::new(Mutex::new(BTreeSet::new()));
    let app = p3_app(RecordGates(seen.clone()));
    block_on(app.call("p3", EXPORT, "coverage", Vec::new())).unwrap();
    block_on(app.call("p3", EXPORT, "exit-success", Vec::new())).unwrap_err();
    block_on(app.call("p3", EXPORT, "exit-code", Vec::new())).unwrap_err();

    assert_eq!(*seen.lock().unwrap(), p3_wit_functions());
}

#[cfg(feature = "wasi-p3")]
fn p3_wit_functions() -> BTreeSet<(String, String)> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success(), "cargo metadata failed");
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let manifests: Vec<_> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|package| package["name"] == "wasmtime-wasi")
        .map(|package| package["manifest_path"].as_str().unwrap())
        .collect();
    assert_eq!(manifests.len(), 1, "expected one resolved wasmtime-wasi");
    let wit = Path::new(manifests[0]).parent().unwrap().join("src/p3/wit");
    let mut resolve = wit_parser::Resolve::default();
    resolve.push_dir(wit).unwrap();

    resolve
        .packages
        .iter()
        .flat_map(|(_, package)| {
            package.interfaces.iter().filter_map(|(name, interface)| {
                let name = package.name.interface_id(name);
                WASI_INTERFACES
                    .contains(&name.as_str())
                    .then_some((name, &resolve.interfaces[*interface]))
            })
        })
        .flat_map(|(interface, definition)| {
            definition
                .functions
                .keys()
                .map(move |function| (interface.clone(), function.clone()))
        })
        .collect()
}

#[test]
#[cfg(feature = "wasi-p3")]
fn ungated_p3_interfaces_remain_missing() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();

    for (package, interface) in [
        ("filesystem", "types"),
        ("sockets", "tcp"),
        ("http", "types"),
    ] {
        let name = format!("wasi:{package}/{interface}@0.3.0");
        let component = Component::from_bytes(component_importing(package, interface))
            .unwrap()
            .named(package);
        let LoadError::MissingImports(missing) = block_on(app.load(component)).unwrap_err() else {
            panic!("expected {name} to remain missing");
        };
        assert_eq!(missing.interfaces(), [name]);
    }
}

#[cfg(feature = "wasi-p3")]
fn component_importing(package: &str, interface: &str) -> Vec<u8> {
    let wit = format!(
        "package wasi:{package}@0.3.0; interface {interface} {{ probe: func(); }} world fixture {{ import {interface}; }}"
    );
    let mut resolve = Resolve::default();
    let package = resolve.push_str("fixture.wit", &wit).unwrap();
    let world = resolve.select_world(&[package], Some("fixture")).unwrap();
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}
