//! End-to-end conformance checks for the native engine.

use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use wasm_junction::{
    App, Call, CallError, CallErrorKind, Caller, Component, Engine, Middleware, Next, Val, Vals,
    WasiConfig,
};
use wasm_junction_conformance::{Fixture, FixtureHost, SUMMARIZER, component, run, sample_note};
use wasm_junction_wasmtime::WasmtimeEngine;

const WASI_COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const ENVIRONMENT: &str = "test:wasi/environment@0.1.0";
const WASI_ENVIRONMENT: &str = "wasi:cli/environment@0.2.12";
const WALL_CLOCK: &str = "wasi:clocks/wall-clock@0.2.12";
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

#[test]
fn successful_scenario_matches_the_engine_neutral_trace() {
    block_on(run(WasmtimeEngine::new().unwrap())).unwrap();
}

#[test]
fn every_call_uses_a_fresh_store() {
    let engine = WasmtimeEngine::new().unwrap();
    let fixture = loaded(&engine);
    block_on(fixture.call("echo", vec![sample_note()])).unwrap();
    block_on(fixture.call("echo", vec![sample_note()])).unwrap();
    assert_eq!(engine.instantiations(), 2);
    assert_eq!(fixture.host().normalizations(), 2);
}

struct EnvironmentBehavior(Arc<Mutex<Vec<Call>>>);

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
fn environment_gate_traces_refuses_and_rewrites() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .wasi(WasiConfig::new().env("GREETING", "hello from WASI"))
        .middleware(EnvironmentBehavior(calls.clone()))
        .build()
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
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 8);
    assert_eq!(calls[0].caller, Caller::Component(Arc::from("wasi")));
    assert_eq!(calls[0].function.as_ref(), "get-environment");
    assert!(calls[0].args.is_empty());
    assert_eq!(calls[4].function.as_ref(), "get-arguments");
    assert_eq!(calls[5].function.as_ref(), "initial-cwd");
    assert_eq!(calls[6].interface.as_ref(), WALL_CLOCK);
    assert_eq!(calls[7].interface.as_ref(), MONOTONIC_CLOCK);
}

#[test]
fn typed_refusal_and_guest_trap_remain_distinct() {
    let fixture = loaded(&WasmtimeEngine::new().unwrap());
    let refusal = block_on(fixture.call("summarize", vec![Val::from("private")])).unwrap();
    assert_eq!(
        refusal,
        [Val::Result(Err(Some(Box::new(Val::from(
            "permission denied"
        )))))]
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
fn malformed_component_has_a_typed_compilation_error() {
    let engine = WasmtimeEngine::new().unwrap();
    let result =
        block_on(engine.compile(Arc::from(&b"not a component"[..]), WasiConfig::default()));
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
