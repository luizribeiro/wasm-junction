use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use wasm_junction::{
    App, AppBuilder, Call, CallContext, CallError, Component, Middleware, Next, Vals,
};
use wasm_junction_wasmtime::WasmtimeEngine;

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dispatch-benchmark.wasm"));

wasm_junction::bindgen!({ path: "benchmark/wit" });

struct Ping;

impl pinger::Host for Ping {
    fn ping(&self, _cx: &CallContext, value: u32) -> u32 {
        value + 1
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
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        next.run(call).await
    }
}

pub(crate) async fn loaded(middleware: Option<Counting>) -> App {
    let builder = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(pinger::provider(Ping));
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
    let runner = app.get::<runner::Runner>("dispatch").unwrap();
    assert_eq!(runner.imports(iterations).await.unwrap(), iterations);
}

pub(crate) async fn noop(app: &App) {
    let runner = app.get::<runner::Runner>("dispatch").unwrap();
    assert_eq!(runner.noop().await.unwrap(), 0);
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
