//! WASI Preview 3 feature and gate behavior.

#![cfg(feature = "wasi")]

use std::future::Future;

#[cfg(not(feature = "wasi-p3"))]
use wasm_junction::LoadError;
use wasm_junction::{App, Component};
use wasm_junction_wasmtime::WasmtimeEngine;

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-p3-test.wasm"));

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
