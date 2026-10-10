use std::sync::Arc;

use wasm_junction_core::{
    BoxFuture, Extensions, ImportDispatcher, ImportTarget, InvocationContext, InvocationId,
    Resource as JunctionResource, Vals, WasiSettings,
};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::WasiCtxBuilder;

use super::{CallError, StoreData};
use crate::wasi::WasiState;

struct Pass;

impl ImportDispatcher for Pass {
    fn call(
        &self,
        _context: InvocationContext,
        _caller: Arc<str>,
        _interface: Arc<str>,
        _function: Arc<str>,
        _args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(async { Err(CallError::trap("unexpected imported call")) })
    }

    fn call_engine(
        &self,
        context: InvocationContext,
        _caller: Arc<str>,
        _interface: Arc<str>,
        _function: Arc<str>,
        args: Vals,
        target: Arc<dyn ImportTarget>,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        target.call(context, args)
    }

    fn drop_resource(
        &self,
        _context: InvocationContext,
        _caller: Arc<str>,
        _resource: JunctionResource,
    ) -> BoxFuture<'_, Result<(), CallError>> {
        Box::pin(async { Ok(()) })
    }
}

pub(super) fn store() -> Store<StoreData> {
    let mut config = Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true);
    let engine = Engine::new(&config).unwrap();
    let mut builder = WasiCtxBuilder::new();
    builder
        .inherit_network()
        .allow_tcp(true)
        .allow_udp(true)
        .allow_ip_name_lookup(true);
    let mut settings = Extensions::default();
    settings.insert(WasiSettings::new());
    let context = InvocationContext::default()
        .with_invocation_id(InvocationId::__from_counter(1))
        .with_settings(settings);
    Store::new(
        &engine,
        StoreData::for_wasi_test(Arc::new(Pass), context, WasiState::new(builder.build())),
    )
}
