//! Differential coverage against wasmtime-wasi's plain linker.

#![cfg(feature = "wasi")]

use wasm_junction::{App, Component as JunctionComponent, Val, WasiSettings};
use wasmtime::component::{Component, Linker, ResourceTable, Val as WasmtimeVal};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const EXPORT: &str = "test:wasi/environment@0.1.0";
#[cfg(feature = "wasi-p3")]
const P3_COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-p3-test.wasm"));
#[cfg(feature = "wasi-p3")]
const P3_EXPORT: &str = "test:wasi-p3/probe@0.1.0";

struct State {
    context: WasiCtx,
    table: ResourceTable,
}

impl State {
    fn configured() -> Self {
        let mut builder = WasiCtxBuilder::new();
        builder.env("GREETING", "hello").arg("alpha");
        Self {
            context: builder.build(),
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

async fn plain() -> String {
    let mut config = Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true);
    let engine = Engine::new(&config).unwrap();
    let component = Component::new(&engine, COMPONENT).unwrap();
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
    let pre = linker.instantiate_pre(&component).unwrap();
    let mut store = Store::new(&engine, State::configured());
    let instance = pre.instantiate_async(&mut store).await.unwrap();
    let interface = instance.get_export_index(&mut store, None, EXPORT).unwrap();
    let function = instance
        .get_export_index(&mut store, Some(&interface), "differential")
        .unwrap();
    let function = instance.get_func(&mut store, function).unwrap();
    let mut results = vec![WasmtimeVal::Bool(false)];
    store
        .run_concurrent(async |accessor| {
            function.call_concurrent(accessor, &[], &mut results).await
        })
        .await
        .unwrap()
        .unwrap();
    let [WasmtimeVal::String(result)] = results.as_slice() else {
        panic!("plain WASI returned the wrong shape")
    };
    result.clone()
}

#[cfg(feature = "wasi-p3")]
async fn plain_p3() -> String {
    let mut config = Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true);
    let engine = Engine::new(&config).unwrap();
    let component = Component::new(&engine, P3_COMPONENT).unwrap();
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p3::add_to_linker(&mut linker).unwrap();
    let pre = linker.instantiate_pre(&component).unwrap();
    let mut store = Store::new(&engine, State::configured());
    let instance = pre.instantiate_async(&mut store).await.unwrap();
    let interface = instance
        .get_export_index(&mut store, None, P3_EXPORT)
        .unwrap();
    let function = instance
        .get_export_index(&mut store, Some(&interface), "coverage")
        .unwrap();
    let function = instance.get_func(&mut store, function).unwrap();
    let mut results = vec![WasmtimeVal::Bool(false)];
    store
        .run_concurrent(async |accessor| {
            function.call_concurrent(accessor, &[], &mut results).await
        })
        .await
        .unwrap()
        .unwrap();
    let [WasmtimeVal::String(result)] = results.as_slice() else {
        panic!("plain Preview 3 WASI returned the wrong shape")
    };
    result.clone()
}

#[test]
fn gated_and_plain_wasi_match_for_standard_interfaces() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    app.configure(
        "gated",
        WasiSettings::new().env("GREETING", "hello").arg("alpha"),
    )
    .unwrap();
    runtime
        .block_on(
            app.load(
                JunctionComponent::from_bytes(COMPONENT)
                    .unwrap()
                    .named("gated"),
            ),
        )
        .unwrap();

    let gated = runtime
        .block_on(app.call("gated", EXPORT, "differential", Vec::new()))
        .unwrap();
    let [Val::String(gated)] = gated.as_slice() else {
        panic!("gated WASI returned the wrong shape")
    };
    assert_eq!(gated, &runtime.block_on(plain()));
}

#[test]
#[cfg(feature = "wasi-p3")]
fn gated_and_plain_wasi_match_for_preview_3_interfaces() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    runtime
        .block_on(
            app.load(
                JunctionComponent::from_bytes(P3_COMPONENT)
                    .unwrap()
                    .named("gated-p3"),
            ),
        )
        .unwrap();

    let gated = runtime
        .block_on(app.call("gated-p3", P3_EXPORT, "coverage", Vec::new()))
        .unwrap();
    let [Val::String(gated)] = gated.as_slice() else {
        panic!("gated Preview 3 WASI returned the wrong shape")
    };
    assert_eq!(gated, &runtime.block_on(plain_p3()));
}
