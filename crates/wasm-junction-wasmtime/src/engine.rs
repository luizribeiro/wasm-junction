use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

#[cfg(feature = "wasi-http")]
use wasm_junction_core::WASI_HTTP_PROVIDER_NAME;
#[cfg(feature = "wasi")]
use wasm_junction_core::WASI_PROVIDER_NAME;
use wasm_junction_core::{
    Access, BoxFuture, CallError, CompiledComponent, Engine, EngineError, FutureHandle,
    ImportDispatcher, InvocationContext, Resource, StreamHandle, Val, Vals, WasiSettings,
    validate_resource_lowering,
};
use wasmtime::component::{
    Component, FutureAny, InstancePre, Linker, ResourceAny, ResourceDynamic, ResourceType,
    Val as WasmtimeVal,
};
use wasmtime::{AsContextMut, Config, Engine as RuntimeEngine, Store};
#[cfg(feature = "wasi")]
use wasmtime_wasi::{FsPerms, WasiCtxBuilder, WasiCtxView, WasiView};
#[cfg(feature = "wasi-http")]
use wasmtime_wasi_http::{WasiHttpCtxView, WasiHttpView, default_hooks};

#[cfg(feature = "wasi-http")]
use crate::WASI_HTTP_INTERFACES;
#[cfg(feature = "wasi")]
use crate::WASI_INTERFACES;
use crate::futures::ActiveFutures;
use crate::imports::{ResourceDefinition, define_imports};
use crate::streams::lower_stream;
use crate::values::{ExpectedResource, LiftValue, LowerValue, from_wasmtime, to_wasmtime};
#[cfg(feature = "wasi")]
use crate::wasi::{WasiState, add_gates};

pub(crate) struct StoreData {
    pub(crate) imports: Arc<dyn ImportDispatcher>,
    pub(crate) context: InvocationContext,
    pub(crate) component: Arc<str>,
    #[cfg(feature = "wasi")]
    wasi: WasiState,
    pub(crate) resources: Arc<[ResourceDefinition]>,
    pub(crate) owned_resources: HashSet<Resource>,
    pub(crate) active_streams: crate::streams::ActiveStreams,
    pub(crate) active_futures: ActiveFutures<FutureAny>,
}

#[cfg(feature = "wasi")]
impl WasiView for StoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi.context,
            table: &mut self.wasi.table,
        }
    }
}

#[cfg(feature = "wasi-http")]
impl WasiHttpView for StoreData {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            hooks: default_hooks(),
            table: &mut self.wasi.table,
            ctx: &mut self.wasi.http,
        }
    }
}

#[cfg(feature = "wasi")]
impl StoreData {
    pub(crate) fn wasi_table(&mut self) -> &mut wasmtime::component::ResourceTable {
        &mut self.wasi.table
    }

    pub(crate) fn open_wasi_channel(&mut self, id: u32) -> bool {
        self.wasi.open_channel(id)
    }

    pub(crate) fn close_wasi_channel(&mut self, id: u32) -> bool {
        self.wasi.close_channel(id)
    }

    pub(crate) fn set_descriptor_preopen(&mut self, id: u32, guest_path: String) {
        self.wasi.set_descriptor_preopen(id, guest_path);
    }

    pub(crate) fn descriptor_preopen(&self, id: u32) -> Option<&str> {
        self.wasi.descriptor_preopen(id)
    }

    pub(crate) fn remove_descriptor_preopen(&mut self, id: u32) {
        self.wasi.remove_descriptor_preopen(id);
    }

    pub(crate) fn set_directory_stream_preopen(&mut self, id: u32, guest_path: String) {
        self.wasi.set_directory_stream_preopen(id, guest_path);
    }

    pub(crate) fn directory_stream_preopen(&self, id: u32) -> Option<&str> {
        self.wasi.directory_stream_preopen(id)
    }

    pub(crate) fn remove_directory_stream_preopen(&mut self, id: u32) {
        self.wasi.remove_directory_stream_preopen(id);
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
        let linker = Linker::new(&engine);
        #[cfg(feature = "wasi")]
        let linker = {
            let mut linker = linker;
            add_gates(&mut linker)?;
            linker
        };
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
    fn provider_interfaces(&self, provider: &str) -> Option<&'static [&'static str]> {
        #[cfg(feature = "wasi")]
        {
            match provider {
                WASI_PROVIDER_NAME => Some(WASI_INTERFACES),
                #[cfg(feature = "wasi-http")]
                WASI_HTTP_PROVIDER_NAME => Some(WASI_HTTP_INTERFACES),
                _ => None,
            }
        }
        #[cfg(not(feature = "wasi"))]
        {
            let _ = provider;
            None
        }
    }

    fn compile(
        &self,
        bytes: Arc<[u8]>,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        let engine = self.engine.clone();
        let linker = self.linker.clone();
        let instantiations = self.instantiations.clone();
        Box::pin(spawn_compile(move || {
            compile_component(&engine, linker, instantiations, bytes)
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
        resources: resources.into(),
    }))
}

struct Compiled {
    pre: InstancePre<StoreData>,
    instantiations: Arc<AtomicU64>,
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
    #[allow(clippy::too_many_lines)]
    async fn call_export(
        &self,
        imports: Arc<dyn ImportDispatcher>,
        context: InvocationContext,
        component: Arc<str>,
        interface: &str,
        function: &str,
        args: Vals,
    ) -> Result<Vals, wasmtime::Error> {
        #[cfg(feature = "wasi")]
        let wasi = wasi_settings(&context);
        let mut store = Store::new(
            self.pre.engine(),
            StoreData {
                imports,
                context,
                component,
                #[cfg(feature = "wasi")]
                wasi: wasi_context(&wasi)?,
                resources: self.resources.clone(),
                owned_resources: HashSet::new(),
                active_streams: Arc::new(std::sync::Mutex::new(HashMap::new())),
                active_futures: ActiveFutures::default(),
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
            let result_types = function_type.results().collect::<Vec<_>>();
            let result_count = result_types.len();
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
                                    LowerValue::Future(future) => accessor
                                        .with(|mut store| lower_future(&future, store.data_mut()))
                                        .map(WasmtimeVal::Future),
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
                        .enumerate()
                        .map(|(index, value)| {
                            from_wasmtime(value, result_types.get(index), &mut |value| match value {
                                LiftValue::Resource(resource) => accessor
                                    .with(|store| lift_resource(resource, store))
                                    .map(Val::Resource),
                                LiftValue::Future(mut future) => accessor.with(|mut store| {
                                    future.close(store.as_context_mut())?;
                                    Err(wasmtime::Error::new(CallError::refused(
                                        "guest-created futures cannot be returned because the Wasmtime store ends with each call",
                                    )))
                                }),
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

pub(crate) fn lift_future(
    future: FutureAny,
    store: &mut StoreData,
) -> Result<FutureHandle, wasmtime::Error> {
    let invocation = store
        .context
        .invocation_id()
        .ok_or_else(|| wasmtime::Error::msg("component future has no invocation id"))?;
    store
        .active_futures
        .insert(future, invocation)
        .map_err(wasmtime::Error::new)
}

pub(crate) fn lower_future(
    future: &FutureHandle,
    store: &mut StoreData,
) -> Result<FutureAny, wasmtime::Error> {
    let invocation = store
        .context
        .invocation_id()
        .ok_or_else(|| wasmtime::Error::msg("component future has no invocation id"))?;
    store
        .active_futures
        .take(future, invocation)
        .map_err(wasmtime::Error::new)
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
    let (runtime_type, expected_interface, expected_name) = {
        let resources = &store.as_context().data().resources;
        let definition = resources
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
        let expected_definition = resources
            .iter()
            .find(|definition| expected.ty == ResourceType::host_dynamic(definition.runtime_type))
            .ok_or_else(|| wasmtime::Error::msg("unknown expected host resource type"))?;
        (
            definition.runtime_type,
            expected_definition.interface.clone(),
            expected_definition.name.clone(),
        )
    };
    let retain = validate_resource_lowering(
        resource,
        &expected_interface,
        &expected_name,
        expected.ownership,
    )
    .map_err(wasmtime::Error::new)?;
    // Constructing a Wasmtime borrow is valid here, but registering it in the host table requires
    // a canonical call scope that does not exist until Wasmtime lowers the declared borrow.
    let dynamic = ResourceDynamic::new_own(resource.id(), runtime_type);
    let dynamic = dynamic.try_into_resource_any(store.as_context_mut())?;
    if retain {
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

#[cfg(feature = "wasi")]
fn wasi_settings(context: &InvocationContext) -> WasiSettings {
    context
        .settings()
        .get::<WasiSettings>()
        .cloned()
        .unwrap_or_default()
}

#[cfg(feature = "wasi")]
fn wasi_context(settings: &WasiSettings) -> wasmtime::Result<WasiState> {
    let mut builder = WasiCtxBuilder::new();
    for (name, value) in settings.environment() {
        builder.env(name, value);
    }
    for argument in settings.arguments() {
        builder.arg(argument);
    }
    for (host_path, guest_path, access) in settings.preopens() {
        let permissions = match access {
            Access::ReadOnly => FsPerms::ReadOnly,
            Access::ReadWrite => FsPerms::ReadWrite,
        };
        builder.preopened_dir(host_path, guest_path, permissions)?;
    }
    Ok(WasiState::new(builder.build()))
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
        block_on(engine.compile(Arc::from(component()))).unwrap();
        assert_eq!(engine.instantiations(), 0);
    }

    #[test]
    fn compiles_imported_host_resources() {
        let engine = WasmtimeEngine::new().unwrap();
        block_on(engine.compile(Arc::from(wasm_junction_conformance::resource_component())))
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
