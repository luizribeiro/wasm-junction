//! Browser-only engine and JavaScript Promise Integration tests.

#![cfg(target_family = "wasm")]

use std::cell::{Cell, RefCell};
use std::future::poll_fn;
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::task::Poll;

use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use wasm_encoder::{CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection};
use wasm_encoder::{ImportSection, Instruction, Module, TypeSection, ValType};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CallErrorKind, CompiledComponent, Component,
    Engine, EngineError, ImportDispatcher, InputStream, InvocationContext, Middleware, Next,
    OutputStream, Provided, Provider, Resource, Val, Vals, WasiConfig,
};
use wasm_junction_conformance::{
    CYCLE_A, DECORATION, DISPATCH_PINGER, DISPATCH_RUNNER, Fixture, FixtureHost, PoisonHost,
    RESOURCE_CLIENT, ReloadGreeter, ReloadHost, ResourceHost, RetainHost, RoutedFixture,
    RoutedHost, STREAM_PROBE, SUMMARIZER, StreamHost, TRANSLATOR, WRITER, component,
    cycle_a_component, cycle_b_component, dispatch_component, reload_v1_component,
    reload_v2_component, resource_component, run_default, run_reload, run_resource_refusal,
    run_resources, run_routed, run_streams, sample_note, sample_summary, stream_component,
    translator_component, writer_component,
};
use wasm_junction_jco::JcoEngine;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

#[wasm_bindgen_test]
fn wasi_provider_is_rejected_during_build() {
    let error = App::builder()
        .provide(wasm_junction::wasi::provider())
        .build()
        .err()
        .unwrap();

    assert!(error.to_string().contains("does not provide WASI yet"));
}

#[derive(Clone)]
struct TrackingEngine {
    inner: JcoEngine,
    compiled: Rc<RefCell<Vec<Weak<dyn CompiledComponent>>>>,
}

impl TrackingEngine {
    fn new() -> Self {
        Self {
            inner: JcoEngine::new(),
            compiled: Rc::default(),
        }
    }

    fn compilation(&self, index: usize) -> Weak<dyn CompiledComponent> {
        self.compiled.borrow()[index].clone()
    }
}

impl Engine for TrackingEngine {
    fn compile(
        &self,
        bytes: Arc<[u8]>,
        wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        Box::pin(async move {
            let compiled = self.inner.compile(bytes, wasi).await?;
            self.compiled.borrow_mut().push(Arc::downgrade(&compiled));
            Ok(compiled)
        })
    }
}
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

#[wasm_bindgen(module = "/js/benchmark.js")]
extern "C" {
    type Heartbeat;

    #[wasm_bindgen(js_name = benchmarkNow)]
    fn benchmark_now() -> f64;

    #[wasm_bindgen(js_name = benchmarkReport)]
    fn benchmark_report(message: &str);

    #[wasm_bindgen(js_name = benchmarkHeartbeatStart)]
    fn benchmark_heartbeat_start() -> Heartbeat;

    #[wasm_bindgen(js_name = benchmarkHeartbeatFinish)]
    async fn benchmark_heartbeat_finish(heartbeat: &Heartbeat, beats: u32) -> f64;
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
async fn refused_import_stops_before_later_host_effects() {
    let fixture = Fixture::new(JcoEngine::new()).await.unwrap();
    let error = fixture
        .call("summarize", vec![Val::from("provider-refusal")])
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "notes provider refused the call");
    assert_eq!(fixture.host().normalizations(), 0);
    assert_eq!(
        fixture.trace().entries(),
        [
            "invocation start summarizer",
            "call host → summarizer example:notes/summarizer@0.1.0.summarize(\"provider-refusal\")",
            "invocation start host",
            "call summarizer → host example:notes/notes@0.1.0.read(\"provider-refusal\")",
            "trap summarizer → host example:notes/notes@0.1.0.read(notes provider refused the call)",
            "invocation end host",
            "trap host → summarizer example:notes/summarizer@0.1.0.summarize(notes provider refused the call)",
            "invocation end summarizer",
        ]
    );
}

#[wasm_bindgen_test]
async fn repeatedly_importing_after_a_refusal_terminates_at_the_next_import() {
    let fixture = Fixture::new(JcoEngine::new()).await.unwrap();
    let error = fixture
        .call("repeat-after-refusal", Vec::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "notes provider refused the call");
    assert_eq!(fixture.host().reads(), 1);
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
async fn default_engine_matches_the_engine_neutral_trace() {
    run_default().await.unwrap();
}

#[wasm_bindgen_test]
async fn routed_scenario_matches_the_engine_neutral_trace() {
    run_routed(JcoEngine::new()).await.unwrap();
}

#[wasm_bindgen_test]
async fn host_resource_calls_cross_middleware_and_the_engine() {
    run_resources(JcoEngine::new()).await.unwrap();
}

#[wasm_bindgen_test]
async fn refused_resource_call_defers_the_drop_to_cleanup() {
    run_resource_refusal(JcoEngine::new()).await.unwrap();
}

#[wasm_bindgen_test]
async fn bidirectional_streams_match_the_engine_neutral_trace() {
    run_streams(JcoEngine::new()).await.unwrap();
}

#[wasm_bindgen_test]
async fn reload_scenario_matches_the_engine_neutral_trace() {
    run_reload(JcoEngine::new()).await.unwrap();
}

#[wasm_bindgen_test]
#[ignore = "run through scripts/browser-bench"]
async fn browser_dispatch_benchmark() {
    const CALLS: u32 = 100_000;
    const INSTANTIATIONS: u32 = 1_000;
    const SAMPLES: usize = 5;

    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(Provided::new(DISPATCH_PINGER, DispatchHost))
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(dispatch_component())
            .unwrap()
            .named("dispatch"),
    )
    .await
    .unwrap();
    dispatch_imports(&app, 1).await;
    dispatch_noop(&app).await;

    let mut imports = Vec::with_capacity(SAMPLES);
    let mut instances = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = benchmark_now();
        dispatch_imports(&app, CALLS).await;
        imports.push((benchmark_now() - start) * 1_000.0 / f64::from(CALLS));

        let start = benchmark_now();
        for _ in 0..INSTANTIATIONS {
            dispatch_noop(&app).await;
        }
        instances.push((benchmark_now() - start) * 1_000.0 / f64::from(INSTANTIATIONS));
    }

    benchmark_report(&format!(
        "host import: {:.1} us/call; samples {imports:.1?}",
        median(&imports)
    ));
    benchmark_report(&format!(
        "fresh instance + noop export: {:.1} us/call; samples {instances:.1?}",
        median(&instances)
    ));
}

struct DispatchHost;

impl Provider for DispatchHost {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            let [Val::U32(value)] = call.args.as_slice() else {
                return Err(CallError::trap("ping expected one u32"));
            };
            Ok(vec![Val::U32(value + 1)])
        })
    }
}

async fn dispatch_imports(app: &App, iterations: u32) {
    let values = app
        .call(
            "dispatch",
            DISPATCH_RUNNER,
            "imports",
            vec![Val::U32(iterations)],
        )
        .await
        .unwrap();
    assert_eq!(values, [Val::U32(iterations)]);
}

async fn dispatch_noop(app: &App) {
    let values = app
        .call("dispatch", DISPATCH_RUNNER, "noop", Vec::new())
        .await
        .unwrap();
    assert_eq!(values, [Val::U32(0)]);
}

#[wasm_bindgen_test]
#[ignore = "run through scripts/browser-bench"]
async fn browser_reload_benchmark() {
    const SAMPLES: usize = 5;

    let host = ReloadHost::default();
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(host.provided())
        .build()
        .unwrap();
    app.load(benchmark_component(reload_v1_component()))
        .await
        .unwrap();

    let mut baseline = Vec::with_capacity(SAMPLES);
    let mut reloads = Vec::with_capacity(SAMPLES);
    let mut pauses = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let idle = benchmark_heartbeat_start();
        baseline.push(benchmark_heartbeat_finish(&idle, 100).await);
        let heartbeat = benchmark_heartbeat_start();
        let bytes = if sample % 2 == 0 {
            reload_v2_component()
        } else {
            reload_v1_component()
        };
        let start = benchmark_now();
        app.reload("greeter", benchmark_component(bytes))
            .await
            .unwrap();
        reloads.push(benchmark_now() - start);
        pauses.push(benchmark_heartbeat_finish(&heartbeat, 1).await);
    }

    benchmark_report(&format!(
        "jco reload: {:.1} ms; samples {reloads:.1?}",
        median(&reloads)
    ));
    benchmark_report(&format!(
        "idle heartbeat gap: {:.3} ms; samples {baseline:.3?}",
        median(&baseline)
    ));
    benchmark_report(&format!(
        "reload heartbeat gap: {:.1} ms; samples {pauses:.1?}",
        median(&pauses)
    ));
}

fn benchmark_component(bytes: &'static [u8]) -> Component {
    Component::from_bytes(bytes).unwrap().named("greeter")
}

fn median(samples: &[f64]) -> f64 {
    let mut samples = samples.to_vec();
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

#[wasm_bindgen_test]
async fn retired_generation_drops_after_its_last_call() {
    let engine = TrackingEngine::new();
    let host = ReloadHost::default();
    let app = App::builder()
        .engine(engine.clone())
        .provide(host.clone().provided())
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(reload_v1_component())
            .unwrap()
            .named("greeter"),
    )
    .await
    .unwrap();
    let old = engine.compilation(0);
    let greeter = app.get::<ReloadGreeter>("greeter").unwrap();
    let mut slow = Box::pin(greeter.greet_slow("Ada"));
    poll_fn(|context| match slow.as_mut().poll(context) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(result) => panic!("slow call ended early: {result:?}"),
    })
    .await;
    host.wait_until_entered().await;

    app.reload(
        "greeter",
        Component::from_bytes(reload_v2_component()).unwrap(),
    )
    .await
    .unwrap();
    assert!(old.upgrade().is_some());
    host.release();
    assert_eq!(slow.await.unwrap(), "v1: hello, Ada");
    assert!(old.upgrade().is_none());
}

#[wasm_bindgen_test]
async fn old_generation_stays_alive_until_its_returned_stream_ends() {
    let engine = TrackingEngine::new();
    let app = App::builder()
        .engine(engine.clone())
        .provide(StreamHost::default().provided())
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(stream_component())
            .unwrap()
            .named("streams"),
    )
    .await
    .unwrap();
    let old = engine.compilation(0);
    let mut returned = app
        .call("streams", STREAM_PROBE, "return-host", Vec::new())
        .await
        .unwrap();
    let stream = InputStream::try_from(returned.remove(0)).unwrap();

    app.reload(
        "streams",
        Component::from_bytes(stream_component()).unwrap(),
    )
    .await
    .unwrap();
    assert!(old.upgrade().is_some());
    assert_eq!(stream.read_all().await.unwrap(), b"Have a good day.");
    assert!(old.upgrade().is_none());
}

#[wasm_bindgen_test]
async fn host_streams_reach_the_guest_and_close_on_early_drop() {
    let host = StreamHost::default();
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(host.clone().provided())
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(stream_component())
            .unwrap()
            .named("streams"),
    )
    .await
    .unwrap();

    let mut nested = app
        .call(
            "streams",
            STREAM_PROBE,
            "echo-optional",
            vec![Val::Option(Some(Box::new(
                OutputStream::from_bytes(b"nested export").into(),
            )))],
        )
        .await
        .unwrap();
    let Val::Option(Some(stream)) = nested.remove(0) else {
        panic!("echo-optional did not return a stream option");
    };
    assert_eq!(
        InputStream::try_from(*stream)
            .unwrap()
            .read_all()
            .await
            .unwrap(),
        b"nested export"
    );

    app.call("streams", STREAM_PROBE, "drop-early", Vec::new())
        .await
        .unwrap();
    assert!(host.reader_closed());
    assert_eq!(
        host.write_error().as_deref(),
        Some("stream reader is closed")
    );

    let error = app
        .call("streams", STREAM_PROBE, "return-guest", Vec::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(error.to_string().contains("store ends with each call"));
}

#[wasm_bindgen_test]
async fn guest_reads_the_first_chunk_before_requesting_the_second() {
    let host = StreamHost::default();
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(host.clone().provided())
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(stream_component())
            .unwrap()
            .named("streams"),
    )
    .await
    .unwrap();
    let result = app
        .call("streams", STREAM_PROBE, "incremental", Vec::new())
        .await
        .unwrap();
    assert_eq!(result, [Val::from("first second")]);
    assert!(host.advanced());
}

#[wasm_bindgen_test]
async fn open_guest_stream_is_aborted_when_its_store_ends() {
    let host = RetainHost::default();
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(host.clone().provided())
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(stream_component())
            .unwrap()
            .named("streams"),
    )
    .await
    .unwrap();
    app.call("streams", STREAM_PROBE, "leave-open", Vec::new())
        .await
        .unwrap();

    assert_eq!(host.first(), b"written");
    let mut input = host.take_input().unwrap();
    assert_eq!(input.read().await.unwrap(), Some(b"in flight".to_vec()));
    assert_eq!(
        input.read().await.unwrap_err().to_string(),
        "stream was aborted when its invocation ended"
    );
}

#[wasm_bindgen_test]
async fn poisoned_invocation_blocks_further_stream_effects() {
    let host = PoisonHost::default();
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(host.clone().provided())
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(stream_component())
            .unwrap()
            .named("streams"),
    )
    .await
    .unwrap();
    let error = app
        .call("streams", STREAM_PROBE, "poison-streams", Vec::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "stream refused");

    let mut guest = host.take_guest().unwrap();
    let source = host.take_source().unwrap();
    assert_eq!(
        guest.read().await.unwrap_err().to_string(),
        "stream was aborted when its invocation ended"
    );
    assert_eq!(
        source.write(b"late").await.unwrap_err().to_string(),
        "stream reader is closed"
    );
    assert_eq!(host.advances(), 0);
}

#[wasm_bindgen_test]
async fn completed_invocation_cleans_up_retained_host_resources() {
    let host = ResourceHost::default();
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(host.clone().provided())
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(resource_component())
            .unwrap()
            .named("resource-client"),
    )
    .await
    .unwrap();
    let result = app
        .call("resource-client", RESOURCE_CLIENT, "retain", Vec::new())
        .await
        .unwrap();
    assert_eq!(result, [Val::from("profile:Grace")]);
    assert_eq!(host.active_resources(), 0);
}

#[wasm_bindgen_test]
async fn resources_cross_guest_exports_as_borrows_and_owned_values() {
    let host = ResourceHost::default();
    let app = resource_app(host.clone()).await;

    let owned = one_resource(
        &call_resource_host(&app, "[constructor]session", vec![Val::from("Lin")]).await,
    );
    let borrowed = Resource::borrowed(owned.interface(), owned.name(), owned.id());
    let inspected = app
        .call(
            "resource-client",
            RESOURCE_CLIENT,
            "inspect",
            vec![Val::Resource(borrowed)],
        )
        .await
        .unwrap();
    assert_eq!(inspected, [Val::from("profile:Lin")]);
    assert_eq!(host.active_resources(), 1);

    let borrowed = Resource::borrowed(owned.interface(), owned.name(), owned.id());
    let error = app
        .call(
            "resource-client",
            RESOURCE_CLIENT,
            "round-trip",
            vec![Val::Resource(borrowed)],
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(error.to_string().contains("requires Own"), "{error}");

    let returned = app
        .call(
            "resource-client",
            RESOURCE_CLIENT,
            "round-trip",
            vec![Val::Resource(owned)],
        )
        .await
        .unwrap();
    let [Val::Resource(returned)] = returned.as_slice() else {
        panic!("guest did not return the owned session")
    };
    drop_host_resource(&app, returned.clone()).await;
    assert_eq!(host.active_resources(), 0);

    let file =
        one_resource(&call_resource_host(&app, "open-file", vec![Val::from("notes.txt")]).await);
    let error = app
        .call(
            "resource-client",
            RESOURCE_CLIENT,
            "round-trip",
            vec![Val::Resource(file.clone())],
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(
        error
            .to_string()
            .contains("does not match the resource type")
    );
    drop_host_resource(&app, file).await;
}

struct WrongResourceResult;

impl Provider for WrongResourceResult {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            match call.function.as_ref() {
                "[constructor]session" => Ok(vec![Val::Resource(Resource::owned(
                    wasm_junction_conformance::RESOURCE_HOST,
                    "file",
                    0,
                ))]),
                function => Err(CallError::unavailable(format!(
                    "wrong resource host has no `{function}` function"
                ))),
            }
        })
    }
}

#[wasm_bindgen_test]
async fn provider_resource_results_match_the_import_signature() {
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(Provided::new(
            wasm_junction_conformance::RESOURCE_HOST,
            WrongResourceResult,
        ))
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(resource_component())
            .unwrap()
            .named("resource-client"),
    )
    .await
    .unwrap();
    let error = app
        .call(
            "resource-client",
            RESOURCE_CLIENT,
            "run",
            vec![Val::Bool(false)],
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(
        error
            .to_string()
            .contains("does not match the resource type")
    );
}

async fn resource_app(host: ResourceHost) -> App {
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(host.provided())
        .build()
        .unwrap();
    app.load(
        Component::from_bytes(resource_component())
            .unwrap()
            .named("resource-client"),
    )
    .await
    .unwrap();
    app
}

async fn call_resource_host(app: &App, function: &str, args: Vals) -> Vals {
    ImportDispatcher::call(
        app,
        InvocationContext::default(),
        Arc::from("resource-client"),
        Arc::from(wasm_junction_conformance::RESOURCE_HOST),
        Arc::from(function),
        args,
    )
    .await
    .unwrap()
}

async fn drop_host_resource(app: &App, resource: Resource) {
    ImportDispatcher::drop_resource(
        app,
        InvocationContext::default(),
        Arc::from("resource-client"),
        resource,
    )
    .await
    .unwrap();
}

fn one_resource(values: &[Val]) -> Resource {
    let [Val::Resource(resource)] = values else {
        panic!("host did not return a resource")
    };
    resource.clone()
}

#[wasm_bindgen_test]
async fn routed_calls_use_fresh_callee_instances() {
    let engine = JcoEngine::new();
    let fixture = RoutedFixture::new(engine.clone()).await.unwrap();
    assert_eq!(
        fixture.write("write", "one").await.unwrap(),
        "host[session=42, hop=writer-to-translator]: one #1"
    );
    assert_eq!(
        fixture.write("write", "two").await.unwrap(),
        "host[session=42, hop=writer-to-translator]: two #1"
    );
    assert_eq!(engine.instantiations(), 4);
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

struct AwaitRoutedCall(Rc<Cell<bool>>);

impl Middleware for AwaitRoutedCall {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.callee.as_ref() == "translator" {
            JsFuture::from(delay(10))
                .await
                .map_err(|error| CallError::trap(format!("timer failed: {error:?}")))?;
            self.0.set(true);
        }
        next.run(call).await
    }
}

struct CountCalls(Rc<Cell<usize>>);

impl Middleware for CountCalls {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        self.0.set(self.0.get() + 1);
        next.run(call).await
    }
}

struct RefuseDecoration(Rc<Cell<usize>>);

impl Middleware for RefuseDecoration {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == DECORATION {
            self.0.set(self.0.get() + 1);
            Err(CallError::refused("decoration refused"))
        } else {
            next.run(call).await
        }
    }
}

#[wasm_bindgen_test]
async fn nested_routed_call_can_await_a_timer() {
    let waited = Rc::new(Cell::new(false));
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(RoutedHost::default().provided())
        .middleware(AwaitRoutedCall(waited.clone()))
        .build()
        .unwrap();
    app.load_all([
        Component::from_bytes(translator_component())
            .unwrap()
            .named("translator"),
        Component::from_bytes(writer_component())
            .unwrap()
            .named("writer"),
    ])
    .await
    .unwrap();
    let result = app
        .call("writer", WRITER, "write-async", vec![Val::from("async")])
        .await
        .unwrap();
    assert_eq!(result, [Val::from("host: async #1")]);
    assert!(waited.get());
}

#[wasm_bindgen_test]
async fn refusal_in_a_routed_callee_stops_its_later_import() {
    let refusals = Rc::new(Cell::new(0));
    let host = RoutedHost::default();
    let app = App::builder()
        .engine(JcoEngine::new())
        .provide(host.clone().provided())
        .middleware(RefuseDecoration(refusals.clone()))
        .build()
        .unwrap();
    app.load_all([
        Component::from_bytes(translator_component())
            .unwrap()
            .named("translator"),
        Component::from_bytes(writer_component())
            .unwrap()
            .named("writer"),
    ])
    .await
    .unwrap();

    let error = app
        .call("writer", WRITER, "write-twice", vec![Val::from("blocked")])
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "decoration refused");
    assert_eq!(refusals.get(), 1);
    assert!(host.callers().is_empty());
}

#[wasm_bindgen_test]
async fn cyclic_routed_calls_stop_at_the_depth_limit_without_leaking_errors() {
    let calls = Rc::new(Cell::new(0));
    let app = App::builder()
        .engine(JcoEngine::new())
        .max_call_depth(3)
        .provide(RoutedHost::default().provided())
        .middleware(CountCalls(calls.clone()))
        .build()
        .unwrap();
    app.load_all([
        Component::from_bytes(cycle_a_component())
            .unwrap()
            .named("a"),
        Component::from_bytes(cycle_b_component())
            .unwrap()
            .named("b"),
        Component::from_bytes(translator_component())
            .unwrap()
            .named("translator"),
        Component::from_bytes(writer_component())
            .unwrap()
            .named("writer"),
    ])
    .await
    .unwrap();

    let error = app
        .call("a", CYCLE_A, "recurse", vec![Val::U32(0)])
        .await
        .unwrap_err();
    assert_eq!(
        error.kind(),
        CallErrorKind::Refused,
        "{error:?}; calls={}",
        calls.get()
    );
    assert_eq!(error.to_string(), "maximum call depth of 3 exceeded");
    assert_eq!(calls.get(), 4);

    let result = app
        .call("writer", WRITER, "write", vec![Val::from("after")])
        .await
        .unwrap();
    assert_eq!(result, [Val::from("host: after #1")]);
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
