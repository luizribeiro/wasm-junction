use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use wasm_junction::{
    App, AppBuilder, BoxFuture, Call, CallContext, Component, Middleware, Next, Provided, Provider,
    Trap, Val, Vals,
};
use wasm_junction_wasmtime::WasmtimeEngine;

const HOST: &str = "benchmark:dispatch/host@0.1.0";
const RUNNER: &str = "benchmark:dispatch/runner@0.1.0";
const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dispatch-benchmark.wasm"));

struct Ping;

impl Provider for Ping {
    fn call<'a>(&'a self, _cx: &'a CallContext, call: Call) -> BoxFuture<'a, Result<Vals, Trap>> {
        Box::pin(async move {
            if call.function.as_ref() != "ping" {
                return Err(Trap::new("benchmark expected host.ping"));
            }
            let [Val::U32(value)] = call.args.as_slice() else {
                return Err(Trap::new("host.ping expected one u32"));
            };
            Ok(vec![Val::U32(value + 1)])
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct Counting(Arc<AtomicU64>);

impl Counting {
    pub(crate) fn calls(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Middleware for Counting {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, Trap> {
        self.0.fetch_add(1, Ordering::Relaxed);
        next.run(call).await
    }
}

pub(crate) async fn loaded(middleware: Option<Counting>) -> App {
    let builder = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(Provided::new(HOST, Ping));
    let app = add_middleware(builder, middleware).build().unwrap();
    app.load(Component::from_bytes(COMPONENT).unwrap().named("dispatch"))
        .await
        .unwrap();
    app
}

fn add_middleware(builder: AppBuilder, middleware: Option<Counting>) -> AppBuilder {
    match middleware {
        Some(middleware) => builder.middleware(middleware),
        None => builder,
    }
}

pub(crate) async fn imports(app: &App, iterations: u32) {
    let values = app
        .call("dispatch", RUNNER, "imports", vec![Val::U32(iterations)])
        .await
        .unwrap();
    assert_eq!(values, [Val::U32(iterations)]);
}

pub(crate) async fn noop(app: &App) {
    let values = app
        .call("dispatch", RUNNER, "noop", Vec::new())
        .await
        .unwrap();
    assert_eq!(values, [Val::U32(0)]);
}

struct ThreadWake(std::thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
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
