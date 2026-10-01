use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use wasm_junction_core::{
    BoxFuture, CallError, CompiledComponent, Engine, EngineError, ImportDispatcher,
    InvocationContext, Vals, WasiConfig,
};
use wasmtime::component::{Component, InstancePre, Linker, Val as WasmtimeVal};
use wasmtime::{Config, Engine as RuntimeEngine, Store};
use wasmtime_wasi::{WasiCtxBuilder, WasiCtxView, WasiView};

use crate::imports::define_imports;
use crate::values::{from_wasmtime, to_wasmtime};
use crate::wasi::{WasiState, add_gates, add_ungated_interfaces};

pub(crate) struct StoreData {
    pub(crate) imports: Arc<dyn ImportDispatcher>,
    pub(crate) context: InvocationContext,
    pub(crate) component: Arc<str>,
    wasi: WasiState,
    pub(crate) gated_wasi: Arc<std::sync::Mutex<WasiState>>,
}

impl WasiView for StoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi.context,
            table: &mut self.wasi.table,
        }
    }
}

/// A native component engine backed by Wasmtime.
#[derive(Clone)]
pub struct WasmtimeEngine {
    engine: RuntimeEngine,
    instantiations: Arc<AtomicU64>,
}

impl WasmtimeEngine {
    /// Creates an engine configured for async component-model calls.
    ///
    /// # Errors
    ///
    /// Returns an error when Wasmtime cannot initialize its compiler or runtime.
    pub fn new() -> Result<Self, wasmtime::Error> {
        let mut config = Config::new();
        config
            .wasm_component_model_async(true)
            .concurrency_support(true);
        Ok(Self {
            engine: RuntimeEngine::new(&config)?,
            instantiations: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Returns the number of stores instantiated for export calls.
    #[must_use]
    pub fn instantiations(&self) -> u64 {
        self.instantiations.load(Ordering::Relaxed)
    }
}

impl Engine for WasmtimeEngine {
    fn supports_import(&self, interface: &str) -> bool {
        interface.starts_with("wasi:")
    }

    fn compile(
        &self,
        bytes: Arc<[u8]>,
        wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        Box::pin(async move {
            let component = Component::new(&self.engine, bytes)
                .map_err(|error| EngineError::new(error.to_string()))?;
            let mut linker = Linker::new(&self.engine);
            add_ungated_interfaces(&mut linker)
                .map_err(|error| EngineError::new(error.to_string()))?;
            add_gates(&mut linker).map_err(|error| EngineError::new(error.to_string()))?;
            define_imports(&mut linker, &component)
                .map_err(|error| EngineError::new(error.to_string()))?;
            let pre = linker
                .instantiate_pre(&component)
                .map_err(|error| EngineError::new(error.to_string()))?;
            Ok(Arc::new(Compiled {
                pre,
                instantiations: self.instantiations.clone(),
                wasi,
            }) as Arc<dyn CompiledComponent>)
        })
    }
}

struct Compiled {
    pre: InstancePre<StoreData>,
    instantiations: Arc<AtomicU64>,
    wasi: WasiConfig,
}

impl CompiledComponent for Compiled {
    fn call(
        &self,
        imports: Arc<dyn ImportDispatcher>,
        context: InvocationContext,
        component: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(async move {
            self.call_export(imports, context, component, &interface, &function, args)
                .await
                .map_err(|error| {
                    error
                        .downcast_ref::<CallError>()
                        .cloned()
                        .unwrap_or_else(|| CallError::trap(format!("{error:#}")))
                })
        })
    }
}

impl Compiled {
    async fn call_export(
        &self,
        imports: Arc<dyn ImportDispatcher>,
        context: InvocationContext,
        component: Arc<str>,
        interface: &str,
        function: &str,
        args: Vals,
    ) -> Result<Vals, wasmtime::Error> {
        let mut store = Store::new(
            self.pre.engine(),
            StoreData {
                imports,
                context,
                component,
                wasi: wasi_context(&self.wasi),
                gated_wasi: Arc::new(std::sync::Mutex::new(wasi_context(&self.wasi))),
            },
        );
        self.instantiations.fetch_add(1, Ordering::Relaxed);
        let instance = self.pre.instantiate_async(&mut store).await?;
        let interface = instance
            .get_export_index(&mut store, None, interface)
            .ok_or_else(|| wasmtime::Error::msg("missing exported interface"))?;
        let function = instance
            .get_export_index(&mut store, Some(&interface), function)
            .ok_or_else(|| wasmtime::Error::msg("missing exported function"))?;
        let function = instance
            .get_func(&mut store, function)
            .ok_or_else(|| wasmtime::Error::msg("export is not a function"))?;
        let params = args
            .into_iter()
            .map(to_wasmtime)
            .collect::<Result<Vec<_>, _>>()?;
        let mut results = vec![WasmtimeVal::Bool(false); function.ty(&store).results().len()];
        store
            .run_concurrent(async |accessor| {
                function
                    .call_concurrent(accessor, &params, &mut results)
                    .await
            })
            .await??;
        results.into_iter().map(from_wasmtime).collect()
    }
}

fn wasi_context(configuration: &WasiConfig) -> WasiState {
    let mut builder = WasiCtxBuilder::new();
    for (name, value) in configuration.environment() {
        builder.env(name, value);
    }
    WasiState::new(builder.build())
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use wasm_junction_conformance::component;

    use super::*;

    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("compilation unexpectedly suspended"),
        }
    }

    #[test]
    fn compiles_component_once_without_instantiating_it() {
        let engine = WasmtimeEngine::new().unwrap();
        ready(engine.compile(Arc::from(component()), WasiConfig::default())).unwrap();
        assert_eq!(engine.instantiations(), 0);
    }
}
