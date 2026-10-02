//! Application builder tests.

mod support;

use std::sync::{Arc, Mutex};

#[cfg(all(feature = "wasi", feature = "wasmtime", not(target_family = "wasm")))]
use support::component_bytes_from;
use support::{NOTES, block_on, component_bytes};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CompiledComponent, Component, Engine,
    EngineError, ImportDispatcher, LoadError, Provided, Provider, Vals, WasiConfig,
};

struct UnusedProvider;

impl Provider for UnusedProvider {
    fn call<'a>(
        &'a self,
        _cx: &'a CallContext,
        _call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async { Err(CallError::trap("unused provider")) })
    }
}

struct FakeEngine;

impl Engine for FakeEngine {
    fn compile(
        &self,
        _bytes: Arc<[u8]>,
        _wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        Box::pin(async { Err(EngineError::new("unused engine")) })
    }
}

fn add_engine(builder: wasm_junction::AppBuilder) -> wasm_junction::AppBuilder {
    builder.engine(FakeEngine)
}

#[test]
fn engine_and_builder_transform_are_accepted() {
    App::builder().apply(add_engine).build().unwrap();
}

struct ConfigEngine(Arc<Mutex<Option<WasiConfig>>>);

impl Engine for ConfigEngine {
    fn compile(
        &self,
        _bytes: Arc<[u8]>,
        wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        *self.0.lock().unwrap() = Some(wasi);
        Box::pin(async { Err(EngineError::new("configuration captured")) })
    }
}

#[test]
fn wasi_configuration_reaches_the_engine_contract() {
    let captured = Arc::new(Mutex::new(None));
    let wasi = WasiConfig::new()
        .env("LANG", "first")
        .env("LANG", "en_US.UTF-8");
    let app = App::builder()
        .engine(ConfigEngine(captured.clone()))
        .wasi(wasi)
        .build()
        .unwrap();
    let bytes = component_bytes("package example:empty@0.1.0; world plugin {}", "plugin");

    let error =
        block_on(app.load(Component::from_bytes(bytes).unwrap().named("empty"))).unwrap_err();

    assert!(matches!(error, LoadError::Compile(_)));
    let configuration = captured.lock().unwrap().take().unwrap();
    assert_eq!(
        configuration.environment().collect::<Vec<_>>(),
        [("LANG", "en_US.UTF-8")]
    );
    assert_eq!(WasiConfig::default().environment().count(), 0);
}

#[cfg(not(any(
    all(feature = "wasmtime", not(target_family = "wasm")),
    all(feature = "jco", target_family = "wasm", target_os = "unknown")
)))]
#[test]
fn build_reports_when_the_target_has_no_default_engine() {
    let error = App::builder().build().err().unwrap();
    let message = error.to_string();
    assert!(message.contains(env!("WASM_JUNCTION_TARGET")));
    assert!(message.contains("call `.engine(…)`"));
}

#[cfg(all(feature = "jco", target_family = "wasm", target_os = "unknown"))]
#[test]
fn build_uses_the_browser_default_engine() {
    App::builder().build().unwrap();
}

#[cfg(all(feature = "wasmtime", not(target_family = "wasm")))]
#[test]
fn build_uses_the_native_default_engine() {
    App::builder().build().unwrap();
}

#[test]
fn duplicate_provider_error_names_both_registration_sites() {
    let first = Provided::new(NOTES, UnusedProvider);
    let first_line = line!() + 1;
    let builder = App::builder().engine(FakeEngine).provide(first);
    let second = Provided::new(NOTES, UnusedProvider);
    let second_line = line!() + 1;
    let error = builder.provide(second).build().err().unwrap();
    let text = error.to_string();

    assert!(text.contains(NOTES));
    assert!(text.contains(&format!("{}:{first_line}", file!())));
    assert!(text.contains(&format!("{}:{second_line}", file!())));
}

#[cfg(all(feature = "wasi", feature = "wasmtime", not(target_family = "wasm")))]
#[test]
fn wasi_provider_conflicts_with_an_app_provider() {
    let duplicate = App::builder()
        .provide(wasm_junction::wasi::provider())
        .provide(wasm_junction::wasi::provider())
        .build()
        .err()
        .unwrap();
    assert!(duplicate.to_string().contains("wasi:cli/environment"));

    let first = wasm_junction::wasi::provider();
    let first_line = line!() + 1;
    let builder = App::builder().provide(first);
    let second = Provided::new("wasi:cli/environment@0.2.12", UnusedProvider);
    let second_line = line!() + 1;
    let error = builder.provide(second).build().err().unwrap();
    let text = error.to_string();

    assert!(text.contains("wasi:cli/environment@0.2.12"));
    assert!(text.contains(&format!("{}:{first_line}", file!())));
    assert!(text.contains(&format!("{}:{second_line}", file!())));
}

#[cfg(all(feature = "wasi", feature = "wasmtime", not(target_family = "wasm")))]
#[test]
fn wasi_preview_three_import_is_missing_with_the_provider() {
    let bytes = component_bytes_from(
        &[
            (
                "wasi-http.wit",
                "package wasi:http@0.3.0; interface client { send: func(); }",
            ),
            (
                "fixture.wit",
                "package test:client@1.0.0; world plugin { import wasi:http/client@0.3.0; }",
            ),
        ],
        "test:client/plugin@1.0.0",
    );
    let app = App::builder()
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();

    let error =
        block_on(app.load(Component::from_bytes(bytes).unwrap().named("client"))).unwrap_err();
    let LoadError::MissingImports(missing) = error else {
        panic!("expected missing imports")
    };
    assert_eq!(missing.interfaces(), ["wasi:http/client@0.3.0"]);
}

fn _engine_contract_is_object_safe(
    engine: &dyn Engine,
    compiled: &dyn CompiledComponent,
    imports: &dyn ImportDispatcher,
) {
    let _ = (engine, compiled, imports);
}
