use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use wasm_junction_core::{
    BoxFuture, CallError, CompiledComponent, Engine, EngineError, ImportDispatcher,
    InvocationContext, Resource, ResourceOwnership, Vals, WasiConfig,
};
use wasmtime::component::{
    Component, InstancePre, Linker, ResourceAny, ResourceDynamic, ResourceType, Val as WasmtimeVal,
};
use wasmtime::{AsContextMut, Config, Engine as RuntimeEngine, Store};
use wasmtime_wasi::{WasiCtxBuilder, WasiCtxView, WasiView};

use crate::imports::{ResourceDefinition, define_imports};
use crate::values::{ExpectedResource, from_wasmtime, to_wasmtime};
use crate::wasi::{WasiState, add_gates, add_ungated_interfaces};

pub(crate) struct StoreData {
    pub(crate) imports: Arc<dyn ImportDispatcher>,
    pub(crate) context: InvocationContext,
    pub(crate) component: Arc<str>,
    wasi: WasiState,
    pub(crate) gated_wasi: Arc<std::sync::Mutex<WasiState>>,
    pub(crate) resources: Arc<[ResourceDefinition]>,
    pub(crate) owned_resources: HashSet<Resource>,
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
            let resources = define_imports(&mut linker, &component)
                .map_err(|error| EngineError::new(error.to_string()))?;
            let pre = linker
                .instantiate_pre(&component)
                .map_err(|error| EngineError::new(error.to_string()))?;
            Ok(Arc::new(Compiled {
                pre,
                instantiations: self.instantiations.clone(),
                wasi,
                resources: resources.into(),
            }) as Arc<dyn CompiledComponent>)
        })
    }
}

struct Compiled {
    pre: InstancePre<StoreData>,
    instantiations: Arc<AtomicU64>,
    wasi: WasiConfig,
    resources: Arc<[ResourceDefinition]>,
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
                resources: self.resources.clone(),
                owned_resources: HashSet::new(),
            },
        );
        let result = async {
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
            let function_type = function.ty(&store);
            let parameter_types = function_type.params().map(|(_, ty)| ty).collect::<Vec<_>>();
            let result_count = function_type.results().len();
            let values = store
                .run_concurrent(async |accessor| {
                    let params = args
                        .into_iter()
                        .enumerate()
                        .map(|(index, value)| {
                            to_wasmtime(
                                value,
                                parameter_types.get(index),
                                &mut |resource, expected| {
                                    accessor
                                        .with(|store| lower_resource(&resource, expected, store))
                                },
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let mut results = vec![WasmtimeVal::Bool(false); result_count];
                    function
                        .call_concurrent(accessor, &params, &mut results)
                        .await?;
                    results
                        .into_iter()
                        .map(|value| {
                            from_wasmtime(value, &mut |resource| {
                                accessor.with(|store| lift_resource(resource, store))
                            })
                        })
                        .collect::<Result<Vals, wasmtime::Error>>()
                })
                .await??;
            Ok(values)
        }
        .await;
        let cleanup = cleanup_resources(&mut store).await;
        match result {
            Ok(values) => cleanup.map(|()| values),
            Err(error) => match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(wasmtime::Error::msg(format!("{error:#}; {cleanup:#}"))),
            },
        }
    }
}

pub(crate) fn lift_resource(
    resource: ResourceAny,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<Resource, wasmtime::Error> {
    let owned = resource.owned();
    let resource = resource.try_into_resource_dynamic(store.as_context_mut())?;
    let definition = store
        .as_context()
        .data()
        .resources
        .get(resource.ty() as usize)
        .filter(|definition| definition.runtime_type == resource.ty())
        .ok_or_else(|| wasmtime::Error::msg("unknown host resource type"))?;
    let make = if owned {
        Resource::owned
    } else {
        Resource::borrowed
    };
    let resource = make(
        definition.interface.clone(),
        definition.name.clone(),
        resource.rep(),
    );
    if owned {
        store
            .as_context_mut()
            .data_mut()
            .owned_resources
            .remove(&resource);
    }
    Ok(resource)
}

pub(crate) fn lower_resource(
    resource: &Resource,
    expected: Option<ExpectedResource>,
    mut store: impl AsContextMut<Data = StoreData>,
) -> Result<ResourceAny, wasmtime::Error> {
    let expected = expected.ok_or_else(|| {
        wasmtime::Error::new(CallError::refused(format!(
            "resource `{}/{}#{}` does not match the declared value type",
            resource.interface(),
            resource.name(),
            resource.id()
        )))
    })?;
    if resource.ownership() != expected.ownership {
        return Err(wasmtime::Error::new(CallError::refused(format!(
            "resource `{}/{}#{}` has {:?} ownership but the call requires {:?} for {:?}",
            resource.interface(),
            resource.name(),
            resource.id(),
            resource.ownership(),
            expected.ownership,
            expected.ty
        ))));
    }
    let definition = store
        .as_context()
        .data()
        .resources
        .iter()
        .find(|definition| {
            definition.interface.as_ref() == resource.interface()
                && definition.name.as_ref() == resource.name()
        })
        .ok_or_else(|| {
            wasmtime::Error::new(CallError::refused(format!(
                "component does not import resource `{}/{}`",
                resource.interface(),
                resource.name()
            )))
        })?;
    if expected.ty != ResourceType::host_dynamic(definition.runtime_type) {
        return Err(wasmtime::Error::new(CallError::refused(format!(
            "resource `{}/{}` does not match the resource type declared by the call",
            resource.interface(),
            resource.name()
        ))));
    }
    // Constructing a Wasmtime borrow is valid here, but registering it in the host table requires
    // a canonical call scope that does not exist until Wasmtime lowers the declared borrow.
    let dynamic = ResourceDynamic::new_own(resource.id(), definition.runtime_type);
    let dynamic = dynamic.try_into_resource_any(store.as_context_mut())?;
    if expected.ownership == ResourceOwnership::Own {
        store
            .as_context_mut()
            .data_mut()
            .owned_resources
            .insert(resource.clone());
    }
    Ok(dynamic)
}

async fn cleanup_resources(store: &mut Store<StoreData>) -> Result<(), wasmtime::Error> {
    let (imports, caller, mut resources) = {
        let data = store.data_mut();
        (
            data.imports.clone(),
            data.component.clone(),
            data.owned_resources.drain().collect::<Vec<_>>(),
        )
    };
    resources.sort_by(|left, right| {
        (left.interface(), left.name(), left.id()).cmp(&(
            right.interface(),
            right.name(),
            right.id(),
        ))
    });
    let mut failures = Vec::new();
    for resource in resources {
        if let Err(error) = imports.drop_resource(caller.clone(), resource).await {
            failures.push(error.to_string());
        }
    }
    if !failures.is_empty() {
        return Err(wasmtime::Error::msg(format!(
            "resource cleanup failed: {}",
            failures.join("; ")
        )));
    }
    Ok(())
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

    #[test]
    fn compiles_imported_host_resources() {
        let engine = WasmtimeEngine::new().unwrap();
        ready(engine.compile(
            Arc::from(wasm_junction_conformance::resource_component()),
            WasiConfig::default(),
        ))
        .unwrap();
    }
}
