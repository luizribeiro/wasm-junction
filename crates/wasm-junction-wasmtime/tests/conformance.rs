//! End-to-end conformance checks for the native engine.

use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use wasm_junction::{
    App, Call, CallError, CallErrorKind, Component, Engine, Middleware, Next, Val, Vals,
};
use wasm_junction_conformance::{Fixture, FixtureHost, SUMMARIZER, component, run, sample_note};
use wasm_junction_wasmtime::WasmtimeEngine;

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
