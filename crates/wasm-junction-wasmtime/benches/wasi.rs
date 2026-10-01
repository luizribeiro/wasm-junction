//! Five-sample release measurements for a WASI environment read.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::hint::black_box;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wasm_junction::{
    App, BoxFuture, CallError, CompiledComponent, Engine, EngineError, ImportDispatcher,
    ImportTarget, InvocationContext, Val, Vals, WasiConfig,
};
use wasmtime::component::ResourceTable;
use wasmtime_wasi::cli::WasiCliView;
use wasmtime_wasi::p2::bindings::cli::environment;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const CALLS: u32 = 100_000;
const SAMPLES: usize = 5;
const INTERFACE: &str = "wasi:cli/environment@0.2.12";

struct State {
    context: WasiCtx,
    table: ResourceTable,
}

impl State {
    fn new() -> Self {
        let mut context = WasiCtxBuilder::new();
        context.env("GREETING", "hello");
        Self {
            context: context.build(),
            table: ResourceTable::new(),
        }
    }

    fn read(&mut self) -> Vec<(String, String)> {
        environment::Host::get_environment(&mut self.cli()).unwrap()
    }
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.context,
            table: &mut self.table,
        }
    }
}

struct EnvironmentTarget(Mutex<State>);

impl ImportTarget for EnvironmentTarget {
    fn call(
        &self,
        _context: InvocationContext,
        args: Vals,
    ) -> BoxFuture<'static, Result<Vals, CallError>> {
        assert!(args.is_empty());
        let values = self.0.lock().unwrap().read();
        Box::pin(async move {
            Ok(vec![Val::List(
                values
                    .into_iter()
                    .map(|(name, value)| Val::Tuple(vec![name.into(), value.into()]))
                    .collect(),
            )])
        })
    }
}

struct UnusedEngine;

impl Engine for UnusedEngine {
    fn compile(
        &self,
        _bytes: Arc<[u8]>,
        _wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        Box::pin(async { Err(EngineError::new("benchmark does not compile components")) })
    }
}

fn main() {
    let mut direct = State::new();
    let app = App::builder().engine(UnusedEngine).build().unwrap();
    let target: Arc<dyn ImportTarget> = Arc::new(EnvironmentTarget(Mutex::new(State::new())));

    let direct_samples = samples(|| {
        for _ in 0..CALLS {
            black_box(direct.read());
        }
    });
    let gated_samples = samples(|| {
        for _ in 0..CALLS {
            let values = futures::executor::block_on(app.call_engine(
                InvocationContext::default(),
                Arc::from("benchmark"),
                Arc::from(INTERFACE),
                Arc::from("get-environment"),
                Vec::new(),
                target.clone(),
            ))
            .unwrap();
            black_box(values);
        }
    });

    print("WASI environment read, ungated", &direct_samples);
    print("WASI environment read, gated", &gated_samples);
}

fn samples(mut run: impl FnMut()) -> Vec<Duration> {
    (0..SAMPLES)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed()
        })
        .collect()
}

fn print(label: &str, samples: &[Duration]) {
    let per_call = |sample: &Duration| sample.as_secs_f64() * 1_000_000_000.0 / f64::from(CALLS);
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let values = samples.iter().map(per_call).collect::<Vec<_>>();
    println!(
        "{label}: {:.1} ns/call; samples {values:.1?}",
        per_call(&ordered[SAMPLES / 2])
    );
}
