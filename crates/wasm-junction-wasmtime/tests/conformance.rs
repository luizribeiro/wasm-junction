//! End-to-end conformance checks for the native engine.

use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CallErrorKind, CompiledComponent, Component,
    Engine, EngineError, ImportDispatcher, InvocationContext, Middleware, Next, Provided, Provider,
    Resource, Val, Vals, WasiSettings,
};
#[cfg(feature = "wasi")]
use wasm_junction::{Caller, LoadError};
use wasm_junction_conformance::{
    CYCLE_A, DECORATION, Fixture, FixtureHost, RELOAD_GREETER, RESOURCE_CLIENT, RESOURCE_HOST,
    ReloadGreeter, ReloadHost, ResourceHost, RoutedFixture, RoutedHost, SUMMARIZER, WRITER,
    component, cycle_a_component, cycle_b_component, reload_v1_component, reload_v2_component,
    resource_component, run, run_reload, run_resource_refusal, run_resources, run_routed,
    sample_note, translator_component, writer_component,
};
use wasm_junction_wasmtime::WasmtimeEngine;

#[cfg(feature = "wasi")]
const WASI_COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
#[cfg(feature = "wasi")]
const ENVIRONMENT: &str = "test:wasi/environment@0.1.0";
#[cfg(feature = "wasi")]
const WASI_ENVIRONMENT: &str = "wasi:cli/environment@0.2.12";
#[cfg(feature = "wasi")]
const WALL_CLOCK: &str = "wasi:clocks/wall-clock@0.2.12";
#[cfg(feature = "wasi")]
const MONOTONIC_CLOCK: &str = "wasi:clocks/monotonic-clock@0.2.12";

struct ThreadWake(std::thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

fn loaded(engine: &WasmtimeEngine) -> Fixture {
    block_on(Fixture::new(engine.clone())).unwrap()
}

fn call_resource_host(app: &App, function: &str, args: Vals) -> Vals {
    block_on(ImportDispatcher::call(
        app,
        InvocationContext::default(),
        Arc::from("resource-client"),
        Arc::from(RESOURCE_HOST),
        Arc::from(function),
        args,
    ))
    .unwrap()
}

fn drop_host_resource(app: &App, resource: Resource) {
    block_on(ImportDispatcher::drop_resource(
        app,
        InvocationContext::default(),
        Arc::from("resource-client"),
        resource,
    ))
    .unwrap();
}

fn one_resource(values: &[Val]) -> Resource {
    let [Val::Resource(resource)] = values else {
        panic!("host did not return a resource")
    };
    resource.clone()
}

#[test]
fn successful_scenario_matches_the_engine_neutral_trace() {
    block_on(run(WasmtimeEngine::new().unwrap())).unwrap();
}

#[test]
fn routed_scenario_matches_the_engine_neutral_trace() {
    block_on(run_routed(WasmtimeEngine::new().unwrap())).unwrap();
}

#[test]
fn refused_resource_call_defers_the_drop_to_cleanup() {
    block_on(run_resource_refusal(WasmtimeEngine::new().unwrap())).unwrap();
}

#[test]
fn reload_scenario_progresses_on_a_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let engine = WasmtimeEngine::new().unwrap();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender.send(runtime.block_on(run_reload(engine))).unwrap();
    });
    receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("reload scenario deadlocked")
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn call_completes_while_reload_compiles_on_a_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(ReloadHost::default().provided())
        .build()
        .unwrap();
    runtime
        .block_on(
            app.load(
                Component::from_bytes(reload_v1_component())
                    .unwrap()
                    .named("greeter"),
            ),
        )
        .unwrap();

    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = runtime.block_on(async {
            let reload = app.reload(
                "greeter",
                Component::from_bytes(reload_v2_component()).unwrap(),
            );
            let mut reload = std::pin::pin!(reload);
            poll_fn(|context| match reload.as_mut().poll(context) {
                Poll::Pending => Poll::Ready(()),
                Poll::Ready(result) => panic!("reload did not suspend: {result:?}"),
            })
            .await;
            let output = app
                .call("greeter", RELOAD_GREETER, "greet", vec![Val::from("Ada")])
                .await
                .map_err(|error| error.to_string())?;
            reload.await.map_err(|error| error.to_string())?;
            Ok::<_, String>(output)
        });
        sender.send(result).unwrap();
    });
    let output = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("concurrent reload deadlocked")
        .unwrap();
    worker.join().unwrap();
    assert_eq!(output, [Val::from("v1: hello, Ada")]);
}

#[derive(Clone)]
struct TrackingEngine {
    inner: WasmtimeEngine,
    compiled: Arc<Mutex<Vec<Weak<dyn CompiledComponent>>>>,
}

impl TrackingEngine {
    fn new() -> Self {
        Self {
            inner: WasmtimeEngine::new().unwrap(),
            compiled: Arc::default(),
        }
    }

    fn compilation(&self, index: usize) -> Weak<dyn CompiledComponent> {
        self.compiled.lock().unwrap()[index].clone()
    }
}

impl Engine for TrackingEngine {
    fn provider_interfaces(&self, provider: &str) -> Option<&'static [&'static str]> {
        self.inner.provider_interfaces(provider)
    }

    fn compile(
        &self,
        bytes: Arc<[u8]>,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        Box::pin(async move {
            let compiled = self.inner.compile(bytes).await?;
            self.compiled
                .lock()
                .unwrap()
                .push(Arc::downgrade(&compiled));
            Ok(compiled)
        })
    }
}

#[test]
fn retired_generation_drops_after_its_last_call() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let engine = TrackingEngine::new();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let old = runtime.block_on(async {
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
            {
                let greeter = app.get::<ReloadGreeter>("greeter").unwrap();
                let slow = greeter.greet_slow("Ada");
                let mut slow = std::pin::pin!(slow);
                poll_fn(|context| match slow.as_mut().poll(context) {
                    Poll::Pending if host.entered() => Poll::Ready(()),
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(result) => panic!("slow call ended early: {result:?}"),
                })
                .await;
                app.reload(
                    "greeter",
                    Component::from_bytes(reload_v2_component()).unwrap(),
                )
                .await
                .unwrap();
                assert!(old.upgrade().is_some());
                host.release();
                assert_eq!(slow.await.unwrap(), "v1: hello, Ada");
            }
            old
        });
        sender.send(old).unwrap();
    });
    let old = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("generation retirement deadlocked");
    worker.join().unwrap();
    assert!(old.upgrade().is_none());
}

#[test]
fn host_resource_calls_cross_middleware_and_the_engine() {
    block_on(run_resources(WasmtimeEngine::new().unwrap())).unwrap();
}

#[test]
fn completed_invocation_cleans_up_retained_host_resources() {
    let host = ResourceHost::default();
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(host.clone().provided())
        .build()
        .unwrap();
    let component = Component::from_bytes(resource_component())
        .unwrap()
        .named("resource-client");
    block_on(app.load(component)).unwrap();
    let result =
        block_on(app.call("resource-client", RESOURCE_CLIENT, "retain", Vec::new())).unwrap();
    assert_eq!(result, [Val::from("profile:Grace")]);
    assert_eq!(host.active_resources(), 0);
}

#[test]
fn resources_cross_guest_exports_as_borrows_and_owned_values() {
    let host = ResourceHost::default();
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(host.clone().provided())
        .build()
        .unwrap();
    let component = Component::from_bytes(resource_component())
        .unwrap()
        .named("resource-client");
    block_on(app.load(component)).unwrap();

    let owned = one_resource(&call_resource_host(
        &app,
        "[constructor]session",
        vec![Val::from("Lin")],
    ));
    let borrowed = wasm_junction::Resource::borrowed(owned.interface(), owned.name(), owned.id());
    let inspected = block_on(app.call(
        "resource-client",
        RESOURCE_CLIENT,
        "inspect",
        vec![Val::Resource(borrowed)],
    ))
    .unwrap();
    assert_eq!(inspected, [Val::from("profile:Lin")]);
    assert_eq!(host.active_resources(), 1);

    let borrowed = Resource::borrowed(owned.interface(), owned.name(), owned.id());
    let error = block_on(app.call(
        "resource-client",
        RESOURCE_CLIENT,
        "round-trip",
        vec![Val::Resource(borrowed)],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(error.to_string().contains("requires Own"), "{error}");
    assert_eq!(host.active_resources(), 1);

    let returned = block_on(app.call(
        "resource-client",
        RESOURCE_CLIENT,
        "round-trip",
        vec![Val::Resource(owned)],
    ))
    .unwrap();
    let [Val::Resource(returned)] = returned.as_slice() else {
        panic!("guest did not return the owned session");
    };
    assert_eq!(host.active_resources(), 1);
    drop_host_resource(&app, returned.clone());
    assert_eq!(host.active_resources(), 0);

    let file = one_resource(&call_resource_host(
        &app,
        "open-file",
        vec![Val::from("notes.txt")],
    ));
    let error = block_on(app.call(
        "resource-client",
        RESOURCE_CLIENT,
        "round-trip",
        vec![Val::Resource(file.clone())],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(
        error
            .to_string()
            .contains("does not match the resource type"),
        "{error}"
    );
    assert_eq!(host.active_resources(), 1);
    drop_host_resource(&app, file);
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
                    RESOURCE_HOST,
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

#[test]
fn provider_resource_results_match_the_import_signature() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(Provided::new(RESOURCE_HOST, WrongResourceResult))
        .build()
        .unwrap();
    let component = Component::from_bytes(resource_component())
        .unwrap()
        .named("resource-client");
    block_on(app.load(component)).unwrap();

    let error = block_on(app.call(
        "resource-client",
        RESOURCE_CLIENT,
        "run",
        vec![Val::Bool(false)],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(
        error
            .to_string()
            .contains("does not match the resource type"),
        "{error}"
    );
}

#[test]
fn routed_calls_use_fresh_callee_instances() {
    let engine = WasmtimeEngine::new().unwrap();
    let fixture = block_on(RoutedFixture::new(engine.clone())).unwrap();
    assert_eq!(
        block_on(fixture.write("write", "one")).unwrap(),
        "host[session=42, hop=writer-to-translator]: one #1"
    );
    assert_eq!(
        block_on(fixture.write("write", "two")).unwrap(),
        "host[session=42, hop=writer-to-translator]: two #1"
    );
    assert_eq!(engine.instantiations(), 4);
}

#[test]
fn every_call_uses_a_fresh_store() {
    let engine = WasmtimeEngine::new().unwrap();
    let fixture = loaded(&engine);
    let output = block_on(fixture.call("echo", vec![sample_note()])).unwrap();
    assert_eq!(output, [sample_note()]);
    block_on(fixture.call("echo", vec![sample_note()])).unwrap();
    assert_eq!(engine.instantiations(), 2);
    assert_eq!(fixture.host().normalizations(), 2);
}

#[cfg(feature = "wasi")]
struct EnvironmentBehavior(Arc<Mutex<Vec<Call>>>);

#[cfg(feature = "wasi")]
impl Middleware for EnvironmentBehavior {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == WALL_CLOCK && call.function.as_ref() == "now" {
            self.0.lock().unwrap().push(call.clone());
            next.run(call).await?;
            return Ok(vec![Val::Record(vec![
                ("seconds".to_owned(), Val::U64(1_700_000_000)),
                ("nanoseconds".to_owned(), Val::U32(123_456_789)),
            ])]);
        }
        if call.interface.as_ref() == MONOTONIC_CLOCK && call.function.as_ref() == "now" {
            self.0.lock().unwrap().push(call.clone());
            next.run(call).await?;
            return Ok(vec![Val::U64(987_654_321)]);
        }
        if call.interface.as_ref() == MONOTONIC_CLOCK
            && call.function.as_ref() == "subscribe-duration"
        {
            self.0.lock().unwrap().push(call.clone());
            return next.run(call).await;
        }
        if call.interface.as_ref() != WASI_ENVIRONMENT {
            return next.run(call).await;
        }
        let number = {
            let mut calls = self.0.lock().unwrap();
            calls.push(call.clone());
            calls.len()
        };
        match number {
            3 => Err(CallError::refused("environment refused")),
            4 => {
                next.run(call).await?;
                Ok(vec![Val::List(vec![Val::Tuple(vec![
                    Val::from("GREETING"),
                    Val::from("fixed"),
                ])])])
            }
            _ => next.run(call).await,
        }
    }
}

#[test]
#[cfg(feature = "wasi")]
fn wasi_imports_are_missing_without_the_provider() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .build()
        .unwrap();
    let component = Component::from_bytes(WASI_COMPONENT).unwrap().named("wasi");
    let mut expected = component.imports().to_vec();
    expected.sort();

    let error = block_on(app.load(component)).unwrap_err();
    let LoadError::MissingImports(missing) = error else {
        panic!("expected missing imports")
    };
    assert_eq!(missing.interfaces(), expected);
}

#[test]
#[cfg(feature = "wasi")]
fn wasi_settings_are_isolated_empty_by_default_and_live() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    app.configure(
        "first",
        WasiSettings::new().env("GREETING", "one").arg("alpha"),
    )
    .unwrap();
    for name in ["first", "second"] {
        block_on(app.load(Component::from_bytes(WASI_COMPONENT).unwrap().named(name))).unwrap();
    }

    let read =
        |name| block_on(app.call(name, ENVIRONMENT, "read", vec![Val::from("GREETING")])).unwrap();
    assert_eq!(
        read("first"),
        [Val::Option(Some(Box::new(Val::from("one"))))]
    );
    assert_eq!(read("second"), [Val::Option(None)]);
    assert_eq!(
        block_on(app.call("first", ENVIRONMENT, "arguments", Vec::new())).unwrap(),
        [Val::List(vec![Val::from("alpha")])]
    );

    app.configure("first", WasiSettings::new().env("GREETING", "two"))
        .unwrap();
    assert_eq!(
        read("first"),
        [Val::Option(Some(Box::new(Val::from("two"))))]
    );
    assert_eq!(read("second"), [Val::Option(None)]);
}

#[test]
#[cfg(feature = "wasi")]
fn environment_gate_traces_refuses_and_rewrites() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(EnvironmentBehavior(calls.clone()))
        .build()
        .unwrap();
    app.configure(
        "wasi",
        WasiSettings::new().env("GREETING", "hello from WASI"),
    )
    .unwrap();
    block_on(app.load(Component::from_bytes(WASI_COMPONENT).unwrap().named("wasi"))).unwrap();

    let configured =
        block_on(app.call("wasi", ENVIRONMENT, "read", vec![Val::from("GREETING")])).unwrap();
    let absent = block_on(app.call("wasi", ENVIRONMENT, "read", vec![Val::from("PATH")])).unwrap();
    let refused =
        block_on(app.call("wasi", ENVIRONMENT, "read", vec![Val::from("GREETING")])).unwrap_err();
    let rewritten =
        block_on(app.call("wasi", ENVIRONMENT, "read", vec![Val::from("GREETING")])).unwrap();
    let arguments = block_on(app.call("wasi", ENVIRONMENT, "arguments", Vec::new())).unwrap();
    let current_directory =
        block_on(app.call("wasi", ENVIRONMENT, "current-directory", Vec::new())).unwrap();
    let wall_time = block_on(app.call("wasi", ENVIRONMENT, "wall-time", Vec::new())).unwrap();
    let monotonic_time =
        block_on(app.call("wasi", ENVIRONMENT, "monotonic-time", Vec::new())).unwrap();
    let timer = block_on(app.call("wasi", ENVIRONMENT, "start-timer", Vec::new())).unwrap();

    assert_eq!(
        configured,
        [Val::Option(Some(Box::new(Val::from("hello from WASI"))))]
    );
    assert_eq!(absent, [Val::Option(None)]);
    assert_eq!(refused.kind(), CallErrorKind::Refused);
    assert_eq!(rewritten, [Val::Option(Some(Box::new(Val::from("fixed"))))]);
    assert_eq!(arguments, [Val::List(Vec::new())]);
    assert_eq!(current_directory, [Val::Option(None)]);
    assert_eq!(
        wall_time,
        [Val::Tuple(vec![
            Val::U64(1_700_000_000),
            Val::U32(123_456_789)
        ])]
    );
    assert_eq!(monotonic_time, [Val::U64(987_654_321)]);
    assert!(timer.is_empty());
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 9);
    assert_eq!(calls[0].caller, Caller::Component(Arc::from("wasi")));
    assert_eq!(calls[0].function.as_ref(), "get-environment");
    assert!(calls[0].args.is_empty());
    assert_eq!(calls[4].function.as_ref(), "get-arguments");
    assert_eq!(calls[5].function.as_ref(), "initial-cwd");
    assert_eq!(calls[6].interface.as_ref(), WALL_CLOCK);
    assert_eq!(calls[7].interface.as_ref(), MONOTONIC_CLOCK);
    assert_eq!(calls[8].function.as_ref(), "subscribe-duration");
    assert_eq!(calls[8].args, [Val::U64(0)]);
}

#[test]
fn wit_error_provider_refusal_and_guest_trap_remain_distinct() {
    let fixture = loaded(&WasmtimeEngine::new().unwrap());
    let refusal = block_on(fixture.call("summarize", vec![Val::from("private")])).unwrap();
    assert_eq!(
        refusal,
        [Val::Result(Err(Some(Box::new(Val::from(
            "permission denied"
        )))))]
    );

    let provider_refusal =
        block_on(fixture.call("summarize", vec![Val::from("provider-refusal")])).unwrap_err();
    assert_eq!(provider_refusal.kind(), CallErrorKind::Refused);
    assert_eq!(
        provider_refusal.to_string(),
        "notes provider refused the call"
    );

    let trap = block_on(fixture.call("crash", Vec::new())).unwrap_err();
    assert_eq!(trap.kind(), CallErrorKind::Trap);
    assert!(
        trap.to_string()
            .contains("example:notes/summarizer@0.1.0#crash"),
        "{trap}"
    );
}

#[test]
fn repeatedly_importing_after_a_refusal_terminates_at_the_next_import() {
    let fixture = loaded(&WasmtimeEngine::new().unwrap());
    let error = block_on(fixture.call("repeat-after-refusal", Vec::new())).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "notes provider refused the call");
    assert_eq!(fixture.host().reads(), 1);
}

#[test]
fn malformed_component_has_a_typed_compilation_error() {
    let engine = WasmtimeEngine::new().unwrap();
    let result = block_on(engine.compile(Arc::from(&b"not a component"[..])));
    let Err(error) = result else {
        panic!("malformed bytes compiled");
    };
    assert!(!error.to_string().is_empty());
}

struct AwaitTimer(Arc<AtomicBool>);

impl Middleware for AwaitTimer {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "normalize" {
            timer(Duration::from_millis(10)).await;
            self.0.store(true, Ordering::Release);
        }
        next.run(call).await
    }
}

#[cfg(feature = "wasi")]
struct AwaitTokioTimer(Arc<AtomicUsize>);

#[cfg(feature = "wasi")]
impl Middleware for AwaitTokioTimer {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if (call.interface.as_ref() == WALL_CLOCK && call.function.as_ref() == "now")
            || (call.interface.as_ref() == WASI_ENVIRONMENT
                && call.function.as_ref() == "initial-cwd")
            || (call.interface.as_ref() == MONOTONIC_CLOCK
                && call.function.as_ref() == "subscribe-duration")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        next.run(call).await
    }
}

struct AwaitRoutedCall;

impl Middleware for AwaitRoutedCall {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.callee.as_ref() == "translator" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        next.run(call).await
    }
}

struct CountCalls(Arc<AtomicUsize>);

impl Middleware for CountCalls {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        next.run(call).await
    }
}

struct RefuseDecoration(Arc<AtomicUsize>);

impl Middleware for RefuseDecoration {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == DECORATION {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(CallError::refused("decoration refused"))
        } else {
            next.run(call).await
        }
    }
}

#[test]
fn nested_routed_call_can_await_on_a_current_thread_tokio_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(RoutedHost::default().provided())
        .middleware(AwaitRoutedCall)
        .build()
        .unwrap();
    let translator = Component::from_bytes(translator_component())
        .unwrap()
        .named("translator");
    let writer = Component::from_bytes(writer_component())
        .unwrap()
        .named("writer");
    runtime
        .block_on(app.load_all([translator, writer]))
        .unwrap();

    let (sender, receiver) = mpsc::channel();
    let _worker = std::thread::spawn(move || {
        let result =
            runtime.block_on(app.call("writer", WRITER, "write-async", vec![Val::from("async")]));
        sender.send(result).unwrap();
    });

    let result = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("nested routed call deadlocked the current-thread Tokio runtime");
    assert_eq!(result.unwrap(), [Val::from("host: async #1")]);
}

#[test]
fn refusal_in_a_routed_callee_stops_its_later_import() {
    let refusals = Arc::new(AtomicUsize::new(0));
    let host = RoutedHost::default();
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(host.clone().provided())
        .middleware(RefuseDecoration(refusals.clone()))
        .build()
        .unwrap();
    let translator = Component::from_bytes(translator_component())
        .unwrap()
        .named("translator");
    let writer = Component::from_bytes(writer_component())
        .unwrap()
        .named("writer");
    block_on(app.load_all([translator, writer])).unwrap();

    let error = block_on(app.call("writer", WRITER, "write-twice", vec![Val::from("blocked")]))
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "decoration refused");
    assert_eq!(refusals.load(Ordering::Relaxed), 1);
    assert!(host.callers().is_empty());
}

#[test]
fn cyclic_routed_calls_stop_at_the_depth_limit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .max_call_depth(3)
        .middleware(CountCalls(calls.clone()))
        .build()
        .unwrap();
    let a = Component::from_bytes(cycle_a_component())
        .unwrap()
        .named("a");
    let b = Component::from_bytes(cycle_b_component())
        .unwrap()
        .named("b");
    block_on(app.load_all([a, b])).unwrap();

    let error = block_on(app.call("a", CYCLE_A, "recurse", vec![Val::U32(0)])).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(
        error
            .to_string()
            .ends_with("maximum call depth of 3 exceeded"),
        "{error}"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 4);
}

#[test]
#[cfg(feature = "wasi")]
fn wasi_gate_can_await_on_a_current_thread_tokio_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let awaits = Arc::new(AtomicUsize::new(0));
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(AwaitTokioTimer(awaits.clone()))
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(WASI_COMPONENT).unwrap().named("wasi")))
        .unwrap();

    let (sender, receiver) = mpsc::channel();
    let _worker = std::thread::spawn(move || {
        let result = runtime.block_on(async {
            app.call("wasi", ENVIRONMENT, "current-directory", Vec::new())
                .await?;
            app.call("wasi", ENVIRONMENT, "start-timer", Vec::new())
                .await?;
            app.call("wasi", ENVIRONMENT, "wall-time", Vec::new()).await
        });
        sender.send(result).unwrap();
    });

    let result = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("WASI middleware deadlocked the current-thread Tokio runtime");
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(awaits.load(Ordering::Relaxed), 3);
}

#[test]
#[cfg(feature = "wasi")]
fn guest_thread_sleep_uses_a_gated_pollable() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(WASI_COMPONENT).unwrap().named("wasi")))
        .unwrap();

    let result = runtime.block_on(app.call("wasi", ENVIRONMENT, "sleep", Vec::new()));
    assert_eq!(result.unwrap(), []);
}

async fn timer(duration: Duration) {
    let state = Arc::new(Mutex::new((false, None::<Waker>)));
    let thread_state = state.clone();
    std::thread::spawn(move || {
        std::thread::sleep(duration);
        let (ready, waker) = &mut *thread_state.lock().unwrap();
        *ready = true;
        if let Some(waker) = waker.take() {
            waker.wake();
        }
    });
    poll_fn(|context| {
        let (ready, waker) = &mut *state.lock().unwrap();
        if *ready {
            Poll::Ready(())
        } else {
            *waker = Some(context.waker().clone());
            Poll::Pending
        }
    })
    .await;
}

#[test]
fn middleware_can_await_a_timer_during_a_plain_import() {
    let waited = Arc::new(AtomicBool::new(false));
    let host = FixtureHost::default();
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(host.provided())
        .middleware(AwaitTimer(waited.clone()))
        .build()
        .unwrap();
    block_on(
        app.load(
            Component::from_bytes(component())
                .unwrap()
                .named("summarizer"),
        ),
    )
    .unwrap();
    let values = block_on(app.call("summarizer", SUMMARIZER, "echo", vec![sample_note()])).unwrap();
    assert_eq!(values, [sample_note()]);
    assert!(waited.load(Ordering::Acquire));
}
