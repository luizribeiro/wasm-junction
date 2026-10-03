//! Application builder tests.

mod support;

use std::sync::Arc;

#[cfg(all(feature = "wasi", feature = "wasmtime", not(target_family = "wasm")))]
use support::component_bytes_from;
use support::{NOTES, block_on};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CompiledComponent, Component, ConfigureError,
    Engine, EngineError, ImportDispatcher, LoadError, Provided, Provider, Vals, WasiSettings,
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

#[test]
fn wasi_settings_require_the_wasi_provider() {
    let app = App::builder().engine(FakeEngine).build().unwrap();

    assert_eq!(
        app.configure("guest", WasiSettings::new().env("TOKEN", "secret")),
        Err(ConfigureError::ProviderNotRegistered { provider: "WASI" })
    );
}

#[cfg(feature = "wasi-http")]
#[test]
fn outgoing_http_provider_requires_engine_support() {
    let error = App::builder()
        .engine(FakeEngine)
        .provide(wasm_junction::wasi::http::provider())
        .build()
        .err()
        .unwrap();

    assert_eq!(
        error,
        wasm_junction::BuildError::UnsupportedEngineProvider {
            provider: "WASI HTTP"
        }
    );
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

#[cfg(all(
    feature = "wasi-http",
    feature = "wasmtime",
    not(target_family = "wasm")
))]
#[test]
fn outgoing_http_provider_is_supported_by_wasmtime() {
    App::builder()
        .provide(wasm_junction::wasi::http::provider())
        .build()
        .unwrap();
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

#[cfg(all(feature = "wasi", feature = "wasmtime", not(target_family = "wasm")))]
#[test]
fn ungated_wasi_families_are_missing_with_the_provider() {
    let bytes = component_bytes_from(
        &[
            (
                "filesystem.wit",
                "package wasi:filesystem@0.2.12; interface types { probe: func(); }",
            ),
            (
                "sockets.wit",
                "package wasi:sockets@0.2.12; interface network { probe: func(); }",
            ),
            (
                "fixture.wit",
                "package test:client@1.0.0; world plugin { import wasi:filesystem/types@0.2.12; import wasi:sockets/network@0.2.12; }",
            ),
        ],
        "test:client/plugin@1.0.0",
    );
    let app = App::builder()
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();

    let error = block_on(app.load(Component::from_bytes(bytes).unwrap().named("client")))
        .expect_err("ungated WASI imports unexpectedly resolved");
    let LoadError::MissingImports(missing) = error else {
        panic!("expected missing imports")
    };
    assert_eq!(
        missing.interfaces(),
        [
            "wasi:filesystem/types@0.2.12",
            "wasi:sockets/network@0.2.12"
        ]
    );
}

fn _engine_contract_is_object_safe(
    engine: &dyn Engine,
    compiled: &dyn CompiledComponent,
    imports: &dyn ImportDispatcher,
) {
    let _ = (engine, compiled, imports);
}
