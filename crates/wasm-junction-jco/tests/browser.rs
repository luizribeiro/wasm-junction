//! Browser-only engine and JavaScript Promise Integration tests.

#![cfg(target_family = "wasm")]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use wasm_encoder::{CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection};
use wasm_encoder::{ImportSection, Instruction, Module, TypeSection, ValType};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CallErrorKind, Component, Engine, Middleware,
    Next, Provided, Provider, Val, Vals, WasiConfig,
};
use wasm_junction_conformance::{
    DECORATION, Fixture, FixtureHost, SUMMARIZER, TRANSLATOR, component, sample_note,
    sample_summary, translator_component,
};
use wasm_junction_jco::JcoEngine;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

#[wasm_bindgen(inline_js = r#"
export async function jspiSmoke(bytes) {
  if (typeof WebAssembly.Suspending !== 'function' ||
      typeof WebAssembly.promising !== 'function') {
    throw new Error('WebAssembly JSPI is unavailable');
  }
  const module = await WebAssembly.compile(bytes);
  const delayed = () => new Promise(resolve => setTimeout(() => resolve(41), 10));
  const instance = new WebAssembly.Instance(module, {
    host: { delayed: new WebAssembly.Suspending(delayed) },
  });
  return WebAssembly.promising(instance.exports.run)();
}

export function delay(milliseconds) {
  return new Promise(resolve => setTimeout(resolve, milliseconds));
}
"#)]
extern "C" {
    #[wasm_bindgen(catch, js_name = jspiSmoke)]
    async fn jspi_smoke(bytes: Uint8Array) -> Result<JsValue, JsValue>;

    fn delay(milliseconds: u32) -> js_sys::Promise;
}

#[wasm_bindgen_test]
async fn suspends_and_resumes_a_core_wasm_call() {
    let result = jspi_smoke(Uint8Array::from(jspi_module().as_slice()))
        .await
        .unwrap();
    assert_eq!(result.as_f64(), Some(42.0));
}

#[wasm_bindgen_test]
async fn awaits_an_import_and_uses_a_fresh_instance() {
    let engine = JcoEngine::new();
    let app = App::builder()
        .engine(engine.clone())
        .provide(Provided::new(DECORATION, DelayedDecoration))
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(translator_component())
            .unwrap()
            .named("translator"),
    )
    .await
    .unwrap();
    for text in ["first", "second"] {
        let result = app
            .call("translator", TRANSLATOR, "translate", vec![Val::from(text)])
            .await
            .unwrap();
        assert_eq!(result, [Val::from(format!("host: {text} #1"))]);
    }
    assert_eq!(engine.instantiations(), 2);
}

#[wasm_bindgen_test]
async fn wit_error_provider_refusal_and_guest_trap_remain_distinct() {
    let fixture = Fixture::new(JcoEngine::new()).await.unwrap();
    let refusal = fixture
        .call("summarize", vec![Val::from("private")])
        .await
        .unwrap();
    assert_eq!(
        refusal,
        [Val::Result(Err(Some(Box::new(Val::from(
            "permission denied"
        )))))]
    );

    let provider_refusal = fixture
        .call("summarize", vec![Val::from("provider-refusal")])
        .await
        .unwrap_err();
    assert_eq!(provider_refusal.kind(), CallErrorKind::Refused);
    assert_eq!(
        provider_refusal.to_string(),
        "notes provider refused the call"
    );

    let trap = fixture.call("crash", Vec::new()).await.unwrap_err();
    assert_eq!(trap.kind(), CallErrorKind::Trap);
    assert!(trap.to_string().contains(&format!("{SUMMARIZER}#crash")));
}

#[wasm_bindgen_test]
async fn provider_error_does_not_leak_into_the_next_call() {
    let fixture = Fixture::new(JcoEngine::new()).await.unwrap();
    let refusal = fixture
        .call("summarize", vec![Val::from("provider-refusal")])
        .await
        .unwrap_err();
    assert_eq!(refusal.kind(), CallErrorKind::Refused);

    let result = fixture
        .call("summarize", vec![Val::from("daily")])
        .await
        .unwrap();
    assert_eq!(result, [sample_summary()]);
}

#[wasm_bindgen_test]
async fn full_note_matches_the_native_echo_result() {
    let fixture = Fixture::new(JcoEngine::new()).await.unwrap();
    let result = fixture.call("echo", vec![sample_note()]).await.unwrap();
    assert_eq!(result, [sample_note()]);
}

#[wasm_bindgen_test]
async fn malformed_component_has_a_typed_compilation_error() {
    let result = JcoEngine::new()
        .compile(Arc::from(&b"not a component"[..]), WasiConfig::default())
        .await;
    let Err(error) = result else {
        panic!("malformed bytes compiled")
    };
    assert!(!error.to_string().is_empty());
}

struct AwaitTimer(Rc<Cell<bool>>);

impl Middleware for AwaitTimer {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "normalize" {
            JsFuture::from(delay(10))
                .await
                .map_err(|error| CallError::trap(format!("timer failed: {error:?}")))?;
            self.0.set(true);
        }
        next.run(call).await
    }
}

#[wasm_bindgen_test]
async fn middleware_can_await_a_timer_during_a_plain_import() {
    let waited = Rc::new(Cell::new(false));
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(FixtureHost::default().provided())
        .middleware(AwaitTimer(waited.clone()))
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(component())
            .unwrap()
            .named("summarizer"),
    )
    .await
    .unwrap();
    let values = app
        .call("summarizer", SUMMARIZER, "echo", vec![sample_note()])
        .await
        .unwrap();
    assert_eq!(values, [sample_note()]);
    assert!(waited.get());
}

struct DelayedDecoration;

impl Provider for DelayedDecoration {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            let [Val::String(text)] = call.args.as_slice() else {
                return Err(CallError::trap("decoration expected one string"));
            };
            JsFuture::from(delay(10))
                .await
                .map_err(|error| CallError::trap(format!("timer failed: {error:?}")))?;
            Ok(vec![Val::from(format!("host: {text}"))])
        })
    }
}

fn jspi_module() -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I32]);
    module.section(&types);
    let mut imports = ImportSection::new();
    imports.import("host", "delayed", EntityType::Function(0));
    module.section(&imports);
    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 1);
    module.section(&exports);
    let mut code = CodeSection::new();
    let mut run = Function::new([]);
    run.instruction(&Instruction::Call(0));
    run.instruction(&Instruction::I32Const(1));
    run.instruction(&Instruction::I32Add);
    run.instruction(&Instruction::End);
    code.function(&run);
    module.section(&code);
    module.finish()
}
