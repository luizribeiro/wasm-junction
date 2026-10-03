//! Five-sample release measurements for 1 MiB of WASI stream writes.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wasm_junction::{App, Component as JunctionComponent, Val};
use wasm_junction_wasmtime::WasmtimeEngine;
use wasmtime::component::{Component, InstancePre, Linker, ResourceTable, Val as WasmtimeVal};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const BYTES: usize = 1024 * 1024;
const SAMPLES: usize = 5;
const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const INTERFACE: &str = "test:wasi/environment@0.1.0";

struct State {
    context: WasiCtx,
    table: ResourceTable,
}

impl State {
    fn new() -> Self {
        Self {
            context: WasiCtxBuilder::new().build(),
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

struct Ungated(InstancePre<State>);

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
        Self(linker.instantiate_pre(&component).unwrap())
    }

    async fn round_trip(&self, bytes: &[u8]) -> Vec<WasmtimeVal> {
        let mut store = Store::new(self.0.engine(), State::new());
        let instance = self.0.instantiate_async(&mut store).await.unwrap();
        let interface = instance
            .get_export_index(&mut store, None, INTERFACE)
            .unwrap();
        let function = instance
            .get_export_index(&mut store, Some(&interface), "benchmark-write")
            .unwrap();
        let function = instance.get_func(&mut store, function).unwrap();
        let params = [WasmtimeVal::List(
            bytes.iter().copied().map(WasmtimeVal::U8).collect(),
        )];
        let mut results = vec![WasmtimeVal::List(Vec::new())];
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
    let bytes = vec![42; BYTES];
    let ungated = Ungated::new();
    let gated = runtime.block_on(async {
        let app = App::builder()
            .engine(WasmtimeEngine::new().unwrap())
            .provide(wasm_junction::wasi::provider())
            .build()
            .unwrap();
        app.load(
            JunctionComponent::from_bytes(Arc::<[u8]>::from(COMPONENT))
                .unwrap()
                .named("streams"),
        )
        .await
        .unwrap();
        app
    });
    black_box(runtime.block_on(ungated.round_trip(&bytes)));
    black_box(runtime.block_on(gated_round_trip(&gated, &bytes)));

    let ungated_samples = samples(|| runtime.block_on(ungated.round_trip(&bytes)));
    let gated_samples = samples(|| runtime.block_on(gated_round_trip(&gated, &bytes)));
    print("ungated", &ungated_samples);
    print("gated", &gated_samples);
}

async fn gated_round_trip(app: &App, bytes: &[u8]) -> Vec<u8> {
    let values = app
        .call(
            "streams",
            INTERFACE,
            "benchmark-write",
            vec![Val::Bytes(bytes.to_vec())],
        )
        .await
        .unwrap();
    let [Val::Bytes(bytes)] = <[Val; 1]>::try_from(values).unwrap() else {
        panic!("benchmark returned the wrong shape")
    };
    bytes
}

fn samples<T>(mut run: impl FnMut() -> T) -> Vec<Duration> {
    (0..SAMPLES)
        .map(|_| {
            let start = Instant::now();
            black_box(run());
            start.elapsed()
        })
        .collect()
}

fn print(label: &str, samples: &[Duration]) {
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let values = samples
        .iter()
        .map(|sample| sample.as_secs_f64() * 1000.0)
        .collect::<Vec<_>>();
    println!(
        "1 MiB in 4 KiB WASI writes, {label}: {:.3} ms; samples {values:.3?}",
        ordered[SAMPLES / 2].as_secs_f64() * 1000.0
    );
}
