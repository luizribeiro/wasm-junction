use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Poll, Waker};

use js_sys::{Array, Uint8Array};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{future_to_promise, spawn_local};
use wasm_junction_core::{
    BoxFuture, CallError, CompiledComponent, Engine, EngineError, ImportDispatcher,
    InvocationContext, OutputStream, OutputStreamWriter, StreamHandle, Vals,
};

use crate::types::Signatures;
use crate::values::{
    JsResult, ResourceTracker, default_result, lift_args_tracked, lift_result_error_tracked,
    lift_result_tracked, lower_args_tracked, lower_result_tracked,
};
use crate::{TranspiledComponent, transpile_component};

#[wasm_bindgen(module = "/js/adapter.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = compileComponent)]
    async fn compile_component(
        source: &str,
        names: &Array,
        modules: &Array,
        resources: &Array,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch)]
    async fn invoke(
        runtime: &JsValue,
        interface: &str,
        function: &str,
        args: &Array,
        dispatch: &js_sys::Function,
        drop_resource: &js_sys::Function,
        read_stream: &js_sys::Function,
        close_stream: &js_sys::Function,
        open_guest_stream: &js_sys::Function,
    ) -> Result<JsValue, JsValue>;

    fn poison(value: JsValue) -> JsValue;

    #[wasm_bindgen(catch, js_name = readGuestStream)]
    async fn read_guest_stream(stream: &JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch, js_name = closeGuestStream)]
    async fn close_guest_stream(stream: &JsValue) -> Result<(), JsValue>;
}

/// A browser component engine backed by jco-generated JavaScript and JSPI.
#[derive(Clone, Default)]
pub struct JcoEngine {
    instantiations: Rc<Cell<u64>>,
}

impl JcoEngine {
    /// Creates a browser component engine.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of fresh instances created for export calls.
    #[must_use]
    pub fn instantiations(&self) -> u64 {
        self.instantiations.get()
    }
}

impl Engine for JcoEngine {
    fn compile(
        &self,
        bytes: Arc<[u8]>,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        let plan = transpile_component(&bytes);
        let instantiations = self.instantiations.clone();
        Box::pin(async move {
            let plan = plan?;
            compile(plan, instantiations).await
        })
    }
}

async fn compile(
    plan: TranspiledComponent,
    instantiations: Rc<Cell<u64>>,
) -> Result<Arc<dyn CompiledComponent>, EngineError> {
    let names = Array::new();
    let modules = Array::new();
    for (name, bytes) in plan.modules {
        names.push(&JsValue::from_str(&name));
        modules.push(&Uint8Array::from(bytes.as_slice()));
    }
    let resources = resource_definitions(&plan.signatures);
    let runtime = compile_component(&plan.source, &names, &modules, &resources)
        .await
        .map_err(|error| EngineError::new(js_error(&error)))?;
    Ok(Arc::new(BrowserCompiled {
        runtime,
        signatures: plan.signatures,
        instantiations,
    }))
}

struct BrowserCompiled {
    runtime: JsValue,
    signatures: Signatures,
    instantiations: Rc<Cell<u64>>,
}

impl CompiledComponent for BrowserCompiled {
    fn call(
        &self,
        imports: Arc<dyn ImportDispatcher>,
        context: InvocationContext,
        component: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        let Some(signature) = self.signatures.export(&interface, &function).cloned() else {
            return Box::pin(std::future::ready(Err(CallError::unavailable(format!(
                "missing component export `{interface}.{function}`"
            )))));
        };
        let resources = ResourceTracker::with_imports(imports.clone(), context.invocation_id());
        let args = lower_args_tracked(args, &signature, &resources);
        Box::pin(async move {
            let args = args?;
            let import_error = Rc::new(RefCell::new(None));
            let bridge = Bridge {
                imports,
                context,
                component,
                signatures: self.signatures.clone(),
                import_error: import_error.clone(),
                resources: resources.clone(),
                guest_streams: Rc::default(),
            };
            let drop_bridge = bridge.clone();
            let read_bridge = bridge.clone();
            let close_bridge = bridge.clone();
            let open_bridge = bridge.clone();
            let cleanup_bridge = bridge.clone();
            let callback = Closure::wrap(Box::new(
                move |interface: String, function: String, args: Array| {
                    let bridge = bridge.clone();
                    future_to_promise(async move {
                        match bridge.dispatch(&interface, &function, &args).await {
                            Ok(JsResult::Return(value)) => Ok(value),
                            Ok(JsResult::Throw(value)) => Err(value),
                            Ok(JsResult::Poison(value)) => Ok(poison(value)),
                            Err(error) => {
                                bridge.remember(error.clone());
                                Err(js_sys::Error::new(&error.to_string()).into())
                            }
                        }
                    })
                },
            )
                as Box<dyn Fn(String, String, Array) -> js_sys::Promise>);
            let drop_callback = Closure::wrap(Box::new(
                move |interface: String, resource: String, id: u32| {
                    let bridge = drop_bridge.clone();
                    future_to_promise(async move {
                        bridge.drop_resource(interface, resource, id).await?;
                        Ok(JsValue::UNDEFINED)
                    })
                },
            )
                as Box<dyn Fn(String, String, u32) -> js_sys::Promise>);
            let read_callback = Closure::wrap(Box::new(move |id: u64| {
                let bridge = read_bridge.clone();
                future_to_promise(async move {
                    bridge.read_host_stream(id).await.map_err(|error| {
                        bridge.remember(error.clone());
                        js_sys::Error::new(&error.to_string()).into()
                    })
                })
            })
                as Box<dyn Fn(u64) -> js_sys::Promise>);
            let close_callback = Closure::wrap(Box::new(move |id: u64| {
                let bridge = close_bridge.clone();
                future_to_promise(async move {
                    bridge.close_host_stream(id);
                    Ok(JsValue::UNDEFINED)
                })
            })
                as Box<dyn Fn(u64) -> js_sys::Promise>);
            let open_callback =
                Closure::wrap(
                    Box::new(move |stream: JsValue| open_bridge.open_guest_stream(stream))
                        as Box<dyn Fn(JsValue) -> JsValue>,
                );
            self.instantiations
                .set(self.instantiations.get().saturating_add(1));
            let result = invoke(
                &self.runtime,
                &interface,
                &function,
                &args,
                callback.as_ref().unchecked_ref(),
                drop_callback.as_ref().unchecked_ref(),
                read_callback.as_ref().unchecked_ref(),
                close_callback.as_ref().unchecked_ref(),
                open_callback.as_ref().unchecked_ref(),
            )
            .await;
            let result = match result {
                Ok(result) => {
                    if let Some(error) = import_error.borrow_mut().take() {
                        Err(error)
                    } else {
                        lift_result_tracked(result, &signature, &resources)
                    }
                }
                Err(error) => {
                    if let Some(error) = import_error.borrow_mut().take() {
                        Err(error)
                    } else if let Some(result) =
                        lift_result_error_tracked(&error, &signature, &resources)?
                    {
                        Ok(result)
                    } else {
                        Err(CallError::trap(format!(
                            "component export `{interface}#{function}` trapped: {}",
                            js_error(&error)
                        )))
                    }
                }
            };
            match (result, cleanup_bridge.cleanup().await) {
                (Ok(values), Ok(())) => Ok(values),
                (Err(error), Ok(())) => Err(error),
                (Ok(_), Err(cleanup)) => Err(cleanup),
                (Err(error), Err(cleanup)) => Err(CallError::trap(format!("{error}; {cleanup}"))),
            }
        })
    }
}

#[derive(Clone)]
struct Bridge {
    imports: Arc<dyn ImportDispatcher>,
    context: InvocationContext,
    component: Arc<str>,
    signatures: Signatures,
    import_error: Rc<RefCell<Option<CallError>>>,
    resources: ResourceTracker,
    guest_streams: Rc<RefCell<HashMap<u64, ActiveGuestStream>>>,
}

#[derive(Clone)]
struct ActiveGuestStream {
    writer: OutputStreamWriter,
    stream: JsValue,
    pump: PumpControl,
}

#[derive(Clone, Default)]
struct PumpControl(Rc<RefCell<PumpState>>);

#[derive(Default)]
struct PumpState {
    cancelled: bool,
    finished: bool,
    reading: bool,
    pump_waker: Option<Waker>,
    finish_waker: Option<Waker>,
}

impl PumpControl {
    async fn read(&self, stream: &JsValue) -> Option<Result<JsValue, JsValue>> {
        let mut read = std::pin::pin!(read_guest_stream(stream));
        poll_fn(|context| {
            let mut state = self.0.borrow_mut();
            if state.cancelled {
                return Poll::Ready(None);
            }
            state.reading = true;
            state.pump_waker = Some(context.waker().clone());
            drop(state);
            read.as_mut().poll(context).map(Some)
        })
        .await
    }

    fn cancel(&self) {
        let waker = {
            let mut state = self.0.borrow_mut();
            state.cancelled = true;
            state.pump_waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn finish(&self) {
        let waker = {
            let mut state = self.0.borrow_mut();
            state.finished = true;
            state.reading = false;
            state.finish_waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    async fn wait(&self) {
        poll_fn(|context| {
            let mut state = self.0.borrow_mut();
            if state.finished {
                Poll::Ready(())
            } else {
                state.finish_waker = Some(context.waker().clone());
                Poll::Pending
            }
        })
        .await;
    }

    #[cfg(test)]
    fn is_reading(&self) -> bool {
        self.0.borrow().reading
    }
}

impl Bridge {
    fn remember(&self, error: CallError) {
        let first = {
            let mut stored = self.import_error.borrow_mut();
            if stored.is_some() {
                false
            } else {
                *stored = Some(error);
                true
            }
        };
        if first {
            self.abort_streams();
        }
    }

    async fn read_host_stream(&self, id: u64) -> Result<JsValue, CallError> {
        if self.import_error.borrow().is_some() {
            return Err(CallError::trap("stream read after import failure"));
        }
        let Some(mut input) = self.resources.checkout_host(id) else {
            return Err(CallError::trap(format!("unknown host stream `{id}`")));
        };
        match input.read().await {
            Ok(Some(bytes)) => {
                self.resources.restore_host(id, input);
                Ok(Uint8Array::from(bytes.as_slice()).into())
            }
            Ok(None) => {
                self.resources.finish_host(id);
                Ok(JsValue::NULL)
            }
            Err(error) => {
                self.resources.finish_host(id);
                Err(CallError::trap(error.to_string()))
            }
        }
    }

    fn close_host_stream(&self, id: u64) {
        if let Some(input) = self.resources.take_host(id) {
            input.close_reader();
        }
    }

    fn open_guest_stream(&self, stream: JsValue) -> JsValue {
        let (writer, output) = OutputStream::channel();
        let handle = StreamHandle::from(output);
        let id = handle.id();
        let marker = self.resources.register_guest(handle);
        let pump = PumpControl::default();
        self.guest_streams.borrow_mut().insert(
            id,
            ActiveGuestStream {
                writer: writer.clone(),
                stream: stream.clone(),
                pump: pump.clone(),
            },
        );
        let active = self.guest_streams.clone();
        let resources = self.resources.clone();
        spawn_local(async move {
            while let Some(Ok(value)) = pump.read(&stream).await {
                if value.is_null() {
                    break;
                }
                let bytes = Uint8Array::new(&value).to_vec();
                if writer.write(bytes).await.is_err() {
                    let _ = close_guest_stream(&stream).await;
                    break;
                }
            }
            if active.borrow_mut().remove(&id).is_some() {
                resources.close_guest(id);
            }
            pump.finish();
        });
        marker
    }

    fn abort_streams(&self) {
        for id in self.resources.host_ids() {
            self.close_host_stream(id);
        }
        for stream in self.guest_streams.borrow().values() {
            stream.writer.abort();
            stream.pump.cancel();
        }
    }

    async fn cleanup(&self) -> Result<(), CallError> {
        self.abort_streams();
        let streams = self.guest_streams.borrow_mut().drain().collect::<Vec<_>>();
        let mut failures = Vec::new();
        for (id, stream) in streams {
            if let Err(error) = close_guest_stream(&stream.stream).await {
                failures.push(js_error(&error));
            }
            stream.pump.wait().await;
            self.resources.close_guest(id);
        }
        if let Err(error) = self.cleanup_resources().await {
            failures.push(error.to_string());
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(CallError::trap(format!(
                "invocation cleanup failed: {}",
                failures.join("; ")
            )))
        }
    }

    async fn dispatch(
        &self,
        interface: &str,
        function: &str,
        args: &Array,
    ) -> Result<JsResult, CallError> {
        let (resolved_interface, signature) = self
            .signatures
            .import(interface, function)
            .map_err(CallError::unavailable)?;
        if self.import_error.borrow().is_some() {
            return default_result(signature);
        }
        let args = lift_args_tracked(args, signature, &self.resources)?;
        let result = self
            .imports
            .call(
                self.context.clone(),
                self.component.clone(),
                Arc::from(resolved_interface),
                Arc::from(function),
                args,
            )
            .await;
        match result {
            Ok(result) => match lower_result_tracked(&result, signature, &self.resources) {
                Ok(result) => Ok(result),
                Err(error) => {
                    self.remember(error);
                    default_result(signature)
                }
            },
            Err(error) => {
                self.remember(error);
                default_result(signature)
            }
        }
    }

    async fn drop_resource(&self, interface: String, name: String, id: u32) -> Result<(), JsValue> {
        if self.import_error.borrow().is_some() {
            return Ok(());
        }
        let resource = self
            .resources
            .take(&interface, &name, id)
            .map_err(|error| {
                self.remember(error.clone());
                js_sys::Error::new(&error.to_string())
            })?;
        self.imports
            .drop_resource(self.context.clone(), self.component.clone(), resource)
            .await
            .map_err(|error| {
                self.remember(error.clone());
                js_sys::Error::new(&error.to_string()).into()
            })
    }

    async fn cleanup_resources(&self) -> Result<(), CallError> {
        let mut resources = self.resources.drain();
        resources.sort_by(|left, right| {
            (left.interface(), left.name(), left.id()).cmp(&(
                right.interface(),
                right.name(),
                right.id(),
            ))
        });
        let mut failures = Vec::new();
        for resource in resources {
            if let Err(error) = self
                .imports
                .drop_resource(self.context.clone(), self.component.clone(), resource)
                .await
            {
                failures.push(error.to_string());
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(CallError::trap(format!(
                "resource cleanup failed: {}",
                failures.join("; ")
            )))
        }
    }
}

fn resource_definitions(signatures: &Signatures) -> Array {
    signatures
        .resources()
        .iter()
        .map(|resource| {
            let definition = Array::new();
            definition.push(&resource.interface.clone().into());
            definition.push(&resource.name.clone().into());
            definition.push(&resource.js_name.clone().into());
            definition.push(
                &resource
                    .constructor
                    .as_deref()
                    .map_or(JsValue::UNDEFINED, JsValue::from_str),
            );
            definition.push(&resource_functions(&resource.methods));
            definition.push(&resource_functions(&resource.statics));
            JsValue::from(definition)
        })
        .collect()
}

fn resource_functions(functions: &[crate::types::ResourceFunction]) -> Array {
    functions
        .iter()
        .map(|function| {
            JsValue::from(Array::of2(
                &JsValue::from_str(&function.wit_name),
                &JsValue::from_str(&function.js_name),
            ))
        })
        .collect()
}

fn js_error(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(value, &"message".into())
                .ok()
                .and_then(|message| message.as_string())
        })
        .unwrap_or_else(|| format!("JavaScript error: {value:?}"))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use js_sys::{Object, Promise, Reflect};
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test;
    use wasm_junction_core::ImportTarget;

    use super::*;
    use crate::types::{FunctionType, ValueType};

    const HOST: &str = "example:test/host@1.0.0";

    #[derive(Default)]
    struct CountingDispatcher(Cell<usize>);

    impl ImportDispatcher for CountingDispatcher {
        fn call(
            &self,
            _context: InvocationContext,
            _caller: Arc<str>,
            _interface: Arc<str>,
            _function: Arc<str>,
            _args: Vals,
        ) -> BoxFuture<'_, Result<Vals, CallError>> {
            self.0.set(self.0.get() + 1);
            Box::pin(std::future::ready(Ok(Vec::new())))
        }

        fn call_engine(
            &self,
            _context: InvocationContext,
            _caller: Arc<str>,
            _interface: Arc<str>,
            _function: Arc<str>,
            _args: Vals,
            _target: Arc<dyn ImportTarget>,
        ) -> BoxFuture<'_, Result<Vals, CallError>> {
            Box::pin(std::future::ready(Ok(Vec::new())))
        }

        fn drop_resource(
            &self,
            _context: InvocationContext,
            _caller: Arc<str>,
            _resource: wasm_junction_core::Resource,
        ) -> BoxFuture<'_, Result<(), CallError>> {
            self.0.set(self.0.get() + 1);
            Box::pin(std::future::ready(Ok(())))
        }
    }

    #[wasm_bindgen_test]
    async fn remembered_failure_blocks_bridge_dispatch_and_drop() {
        let dispatcher = Arc::new(CountingDispatcher::default());
        let bridge = Bridge {
            imports: dispatcher.clone(),
            context: InvocationContext::default(),
            component: Arc::from("resource-client"),
            signatures: Signatures::for_test_import(
                HOST,
                "read",
                FunctionType {
                    params: vec![ValueType::String],
                    result: Some(ValueType::U32),
                },
            ),
            import_error: Rc::new(RefCell::new(Some(CallError::refused("denied")))),
            resources: ResourceTracker::default(),
            guest_streams: Rc::default(),
        };
        let args = Array::of1(&JsValue::from_str("Ada"));
        assert!(matches!(
            bridge.dispatch(HOST, "read", &args).await.unwrap(),
            JsResult::Poison(_)
        ));
        bridge
            .drop_resource(HOST.to_owned(), "session".to_owned(), 7)
            .await
            .unwrap();
        assert_eq!(dispatcher.0.get(), 0);
    }

    #[wasm_bindgen_test]
    async fn cleanup_ends_a_pump_blocked_in_a_guest_read() {
        let dispatcher = Arc::new(CountingDispatcher::default());
        let bridge = Bridge {
            imports: dispatcher,
            context: InvocationContext::default(),
            component: Arc::from("streams"),
            signatures: Signatures::for_test_import(
                HOST,
                "read",
                FunctionType {
                    params: Vec::new(),
                    result: None,
                },
            ),
            import_error: Rc::default(),
            resources: ResourceTracker::default(),
            guest_streams: Rc::default(),
        };
        let stream = Object::new();
        let read =
            Closure::wrap(
                Box::new(|_options: JsValue| Promise::new(&mut |_resolve, _reject| {}))
                    as Box<dyn FnMut(JsValue) -> Promise>,
            );
        Reflect::set(&stream, &"read".into(), read.as_ref()).unwrap();
        let closed = Rc::new(Cell::new(false));
        let close_state = closed.clone();
        let close = Closure::wrap(Box::new(move || {
            close_state.set(true);
            Promise::resolve(&JsValue::UNDEFINED)
        }) as Box<dyn FnMut() -> Promise>);
        Reflect::set(&stream, &"return".into(), close.as_ref()).unwrap();

        bridge.open_guest_stream(stream.into());
        let pump = bridge
            .guest_streams
            .borrow()
            .values()
            .next()
            .unwrap()
            .pump
            .clone();
        for _ in 0..10 {
            if pump.is_reading() {
                break;
            }
            JsFuture::from(Promise::resolve(&JsValue::UNDEFINED))
                .await
                .unwrap();
        }
        assert!(pump.is_reading());
        let active = Rc::downgrade(&bridge.guest_streams);

        bridge.cleanup().await.unwrap();
        assert!(closed.get());
        drop(bridge);
        assert!(active.upgrade().is_none());
    }
}
