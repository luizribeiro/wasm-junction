//! WASI Preview 3 feature and gate behavior.

#![cfg(feature = "wasi")]

use std::future::Future;
#[cfg(feature = "wasi-p3")]
use std::sync::Arc;
#[cfg(feature = "wasi-p3")]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(not(feature = "wasi-p3"))]
use wasm_junction::LoadError;
use wasm_junction::{App, Component};
#[cfg(feature = "wasi-p3")]
use wasm_junction::{Call, CallError, CallErrorKind, Middleware, Next, Val, Vals};
use wasm_junction_wasmtime::WasmtimeEngine;

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
