//! Five-sample Wasmtime reload and concurrent-call measurement.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use wasm_junction::{App, Component, Val};
use wasm_junction_conformance::{
    RELOAD_GREETER, ReloadHost, reload_v1_component, reload_v2_component,
};
use wasm_junction_wasmtime::WasmtimeEngine;

const SAMPLES: usize = 5;

fn main() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(ReloadHost::default().provided())
        .build()
        .unwrap();
    block_on(app.load(component(reload_v1_component(), "greeter"))).unwrap();
    call(&app);

    let active = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let concurrent = Arc::new(Mutex::new(vec![Vec::new(); SAMPLES]));
    let runner = {
        let app = app.clone();
        let active = active.clone();
        let completed = completed.clone();
        let stop = stop.clone();
        let concurrent = concurrent.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let sample = active.load(Ordering::Acquire);
                let start = Instant::now();
                call(&app);
                let elapsed = start.elapsed();
                completed.fetch_add(1, Ordering::Release);
                if sample > 0 && sample == active.load(Ordering::Acquire) {
                    concurrent.lock().unwrap()[sample - 1].push(elapsed);
                }
            }
        })
    };
    while completed.load(Ordering::Acquire) < 10 {
        std::thread::yield_now();
    }

    let mut reloads = Vec::new();
    for sample in 0..SAMPLES {
        active.store(sample + 1, Ordering::Release);
        let bytes = if sample % 2 == 0 {
            reload_v2_component()
        } else {
            reload_v1_component()
        };
        let replacement = component(bytes, "unused");
        let start = Instant::now();
        block_on(app.reload("greeter", replacement)).unwrap();
        reloads.push(start.elapsed());
        active.store(0, Ordering::Release);
    }
    stop.store(true, Ordering::Release);
    runner.join().unwrap();

    let concurrent = concurrent.lock().unwrap();
    assert!(concurrent.iter().all(|sample| !sample.is_empty()));
    let during = concurrent
        .iter()
        .map(|sample| median(sample))
        .collect::<Vec<_>>();
    println!(
        "Wasmtime compile + swap: {:.1} ms; samples {:.1?}",
        millis(median(&reloads)),
        reloads
            .iter()
            .map(|value| millis(*value))
            .collect::<Vec<_>>()
    );
    println!(
        "call during reload: {:.1} us; medians {:.1?}; calls {:?}",
        micros(median(&during)),
        during
            .iter()
            .map(|value| micros(*value))
            .collect::<Vec<_>>(),
        concurrent.iter().map(Vec::len).collect::<Vec<_>>()
    );
}

fn component(bytes: &'static [u8], name: &str) -> Component {
    Component::from_bytes(bytes).unwrap().named(name)
}

fn call(app: &App) {
    let output =
        block_on(app.call("greeter", RELOAD_GREETER, "greet", vec![Val::from("Ada")])).unwrap();
    assert!(matches!(output.as_slice(), [Val::String(_)]));
}

fn median(samples: &[Duration]) -> Duration {
    let mut samples = samples.to_vec();
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

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
