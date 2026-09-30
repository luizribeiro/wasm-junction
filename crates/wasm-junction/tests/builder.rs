//! Application builder tests.

mod support;

use std::sync::Arc;

use support::NOTES;
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CompiledComponent, Engine, EngineError, ImportDispatcher,
    Provided, Provider, Trap, Vals,
};

struct UnusedProvider;

impl Provider for UnusedProvider {
    fn call<'a>(&'a self, _cx: &'a CallContext, _call: Call) -> BoxFuture<'a, Result<Vals, Trap>> {
        Box::pin(async { Err(Trap::new("unused provider")) })
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
fn build_requires_an_engine() {
    let error = App::builder().build().err().unwrap();
    assert_eq!(error.to_string(), "an engine is required");
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

fn _engine_contract_is_object_safe(
    engine: &dyn Engine,
    compiled: &dyn CompiledComponent,
    imports: &dyn ImportDispatcher,
) {
    let _ = (engine, compiled, imports);
}
