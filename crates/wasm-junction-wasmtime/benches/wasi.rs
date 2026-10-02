//! Five-sample release measurements for a guest WASI environment read.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wasm_junction::{App, Component as JunctionComponent, Val, WasiSettings};
use wasm_junction_wasmtime::WasmtimeEngine;
use wasmtime::component::{Component, InstancePre, Linker, ResourceTable, Val as WasmtimeVal};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const CALLS: u32 = 10_000;
const SAMPLES: usize = 5;
const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const INTERFACE: &str = "test:wasi/environment@0.1.0";

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
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.context,
            table: &mut self.table,
        }
    }
}

struct Ungated {
    pre: InstancePre<State>,
}

impl Ungated {
    fn new() -> Self {
        let mut config = Config::new();
        config
            .wasm_component_model_async(true)
            .concurrency_support(true);
        let engine = Engine::new(&config).unwrap();
        let component = Component::new(&engine, COMPONENT).unwrap();
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
        Self {
            pre: linker.instantiate_pre(&component).unwrap(),
        }
    }

    async fn read(&self) -> Vec<WasmtimeVal> {
        let mut store = Store::new(self.pre.engine(), State::new());
        let instance = self.pre.instantiate_async(&mut store).await.unwrap();
        let interface = instance
            .get_export_index(&mut store, None, INTERFACE)
            .unwrap();
        let function = instance
            .get_export_index(&mut store, Some(&interface), "read")
            .unwrap();
        let function = instance.get_func(&mut store, function).unwrap();
        let params = [WasmtimeVal::String("GREETING".into())];
        let mut results = vec![WasmtimeVal::Bool(false)];
        store
            .run_concurrent(async |accessor| {
                function
                    .call_concurrent(accessor, &params, &mut results)
                    .await
            })
            .await
            .unwrap()
            .unwrap();
        results
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let ungated = Ungated::new();
    let gated = runtime.block_on(async {
        let app = App::builder()
            .engine(WasmtimeEngine::new().unwrap())
            .provide(wasm_junction::wasi::provider())
            .build()
            .unwrap();
        app.configure("wasi", WasiSettings::new().env("GREETING", "hello"))
            .unwrap();
        app.load(
            JunctionComponent::from_bytes(Arc::<[u8]>::from(COMPONENT))
                .unwrap()
                .named("wasi"),
        )
        .await
        .unwrap();
        app
    });

    let ungated_samples = samples(|| {
        for _ in 0..CALLS {
            black_box(runtime.block_on(ungated.read()));
        }
    });
    let gated_samples = samples(|| {
        for _ in 0..CALLS {
            let values = runtime
                .block_on(gated.call("wasi", INTERFACE, "read", vec![Val::from("GREETING")]))
                .unwrap();
            black_box(values);
        }
    });

    print("WASI guest environment read, ungated", &ungated_samples);
    print("WASI guest environment read, gated", &gated_samples);
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
