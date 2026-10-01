use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use wasm_junction_core::{
    BoxFuture, CallError, CompiledComponent, Engine, EngineError, ImportDispatcher,
    InvocationContext, Resource, ResourceOwnership, StreamHandle, Val, Vals, WasiConfig,
};
use wasmtime::component::{
    Component, InstancePre, Linker, ResourceAny, ResourceDynamic, ResourceType, Val as WasmtimeVal,
};
use wasmtime::{AsContextMut, Config, Engine as RuntimeEngine, Store};
use wasmtime_wasi::{WasiCtxBuilder, WasiCtxView, WasiView};

use crate::imports::{ResourceDefinition, define_imports};
use crate::streams::lower_stream;
use crate::values::{ExpectedResource, LiftValue, LowerValue, from_wasmtime, to_wasmtime};
use crate::wasi::{WasiState, add_gates, add_ungated_interfaces};

pub(crate) struct StoreData {
    pub(crate) imports: Arc<dyn ImportDispatcher>,
    pub(crate) context: InvocationContext,
    pub(crate) component: Arc<str>,
    wasi: WasiState,
    pub(crate) gated_wasi: Arc<std::sync::Mutex<WasiState>>,
    pub(crate) resources: Arc<[ResourceDefinition]>,
    pub(crate) owned_resources: HashSet<Resource>,
    pub(crate) active_streams: crate::streams::ActiveStreams,
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
    linker: Linker<StoreData>,
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
        let engine = RuntimeEngine::new(&config)?;
        let mut linker = Linker::new(&engine);
        add_ungated_interfaces(&mut linker)?;
        add_gates(&mut linker)?;
        Ok(Self {
            engine,
            linker,
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
        let engine = self.engine.clone();
        let linker = self.linker.clone();
        let instantiations = self.instantiations.clone();
        Box::pin(spawn_compile(move || {
            compile_component(&engine, linker, instantiations, bytes, wasi)
        }))
    }
}

fn spawn_compile(compile: impl FnOnce() -> CompileResult + Send + 'static) -> CompileReceiver {
    let (channel, receiver) = compile_channel();
    let worker = channel.clone();
    let run = move || {
        let result = catch_unwind(AssertUnwindSafe(compile)).unwrap_or_else(|panic| {
            Err(EngineError::new(format!(
                "compilation panicked: {}",
                panic_message(panic.as_ref())
            )))
        });
        send_compile(&worker, result);
    };
    if let Err(error) = std::thread::Builder::new()
        .name("wasm-junction-compile".to_owned())
        .spawn(run)
    {
        send_compile(
            &channel,
            Err(EngineError::new(format!(
                "could not start compilation worker: {error}"
            ))),
        );
    }
    receiver
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = panic.downcast_ref::<&str>() {
        message
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message
    } else {
        "unknown panic"
    }
}

type CompileResult = Result<Arc<dyn CompiledComponent>, EngineError>;

#[derive(Default)]
struct CompileChannel {
    result: Option<CompileResult>,
    waker: Option<Waker>,
}

struct CompileReceiver(Arc<Mutex<CompileChannel>>);

impl Future for CompileReceiver {
    type Output = CompileResult;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut channel = lock_channel(&self.0);
        if let Some(result) = channel.result.take() {
            Poll::Ready(result)
        } else {
            channel.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }
}

fn compile_channel() -> (Arc<Mutex<CompileChannel>>, CompileReceiver) {
    let channel = Arc::new(Mutex::new(CompileChannel::default()));
    (channel.clone(), CompileReceiver(channel))
}

fn send_compile(channel: &Mutex<CompileChannel>, result: CompileResult) {
    let waker = {
        let mut channel = lock_channel(channel);
        channel.result = Some(result);
        channel.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

fn lock_channel(channel: &Mutex<CompileChannel>) -> std::sync::MutexGuard<'_, CompileChannel> {
    match channel.lock() {
        Ok(channel) => channel,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn compile_component(
    engine: &RuntimeEngine,
    mut linker: Linker<StoreData>,
    instantiations: Arc<AtomicU64>,
    bytes: Arc<[u8]>,
    wasi: WasiConfig,
) -> CompileResult {
    let component =
        Component::new(engine, bytes).map_err(|error| EngineError::new(error.to_string()))?;
    let resources = define_imports(&mut linker, &component)
        .map_err(|error| EngineError::new(error.to_string()))?;
    let pre = linker
        .instantiate_pre(&component)
        .map_err(|error| EngineError::new(error.to_string()))?;
    Ok(Arc::new(Compiled {
        pre,
        instantiations,
        wasi,
        resources: resources.into(),
    }))
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
                active_streams: Arc::new(std::sync::Mutex::new(HashMap::new())),
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
                                &mut |value| match value {
                                    LowerValue::Resource(resource, expected) => accessor
                                        .with(|store| {
                                            lower_resource(&resource, expected, store)
                                        })
                                        .map(WasmtimeVal::Resource),
                                    LowerValue::Stream(stream) => accessor
                                        .with(|store| lower_stream(stream, store))
                                        .map(WasmtimeVal::Stream),
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
                            from_wasmtime(value, &mut |value| match value {
                                LiftValue::Resource(resource) => accessor
                                    .with(|store| lift_resource(resource, store))
                                    .map(Val::Resource),
                                LiftValue::Stream(stream) => {
                                    let reader = stream.try_into_stream_reader::<u8>()?;
                                    accessor.with(|mut store| {
                                        match reader
                                            .try_into::<StreamHandle>(store.as_context_mut())
                                        {
                                            Ok(handle) => Ok(Val::Stream(handle)),
                                            Err(mut reader) => {
                                                reader.close(store.as_context_mut())?;
                                                Err(wasmtime::Error::new(CallError::refused(
                                                    "guest-created streams cannot be returned because the Wasmtime store ends with each call",
                                                )))
                                            }
                                        }
                                    })
                                }
                            })
                        })
                        .collect::<Result<Vals, wasmtime::Error>>()
                })
                .await??;
            Ok(values)
        }
        .await;
        crate::streams::abort_streams(store.data());
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
    let (imports, context, caller, mut resources) = {
        let data = store.data_mut();
        (
            data.imports.clone(),
            data.context.clone(),
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
        if let Err(error) = imports
            .drop_resource(context.clone(), caller.clone(), resource)
            .await
        {
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
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::Duration;

    use wasm_junction_conformance::component;

    use super::*;

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

    #[test]
    fn compiles_component_once_without_instantiating_it() {
        let engine = WasmtimeEngine::new().unwrap();
        block_on(engine.compile(Arc::from(component()), WasiConfig::default())).unwrap();
        assert_eq!(engine.instantiations(), 0);
    }

    #[test]
    fn compiles_imported_host_resources() {
        let engine = WasmtimeEngine::new().unwrap();
        block_on(engine.compile(
            Arc::from(wasm_junction_conformance::resource_component()),
            WasiConfig::default(),
        ))
        .unwrap();
    }

    #[test]
    fn reports_a_compilation_worker_panic() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender
                .send(block_on(spawn_compile(|| {
                    panic!("deliberate compilation panic")
                })))
                .unwrap();
        });

        let result = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("panicking compilation left its receiver pending");
        worker.join().unwrap();
        let Err(error) = result else {
            panic!("panicking compilation unexpectedly succeeded");
        };
        assert_eq!(
            error.to_string(),
            "compilation panicked: deliberate compilation panic"
        );
    }
}
