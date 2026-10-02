use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use js_sys::{Array, Uint8Array};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::future_to_promise;
use wasm_junction_core::{
    BoxFuture, CallError, CompiledComponent, Engine, EngineError, ImportDispatcher,
    InvocationContext, Vals, WasiConfig,
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
    ) -> Result<JsValue, JsValue>;
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
        _wasi: WasiConfig,
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
        let resources = ResourceTracker::default();
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
            };
            let drop_bridge = bridge.clone();
            let cleanup_bridge = bridge.clone();
            let callback = Closure::wrap(Box::new(
                move |interface: String, function: String, args: Array| {
                    let bridge = bridge.clone();
                    future_to_promise(async move {
                        match bridge.dispatch(&interface, &function, &args).await {
                            Ok(JsResult::Return(value)) => Ok(value),
                            Ok(JsResult::Throw(value)) => Err(value),
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
            self.instantiations
                .set(self.instantiations.get().saturating_add(1));
            let result = invoke(
                &self.runtime,
                &interface,
                &function,
                &args,
                callback.as_ref().unchecked_ref(),
                drop_callback.as_ref().unchecked_ref(),
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
            match (result, cleanup_bridge.cleanup_resources().await) {
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
}

impl Bridge {
    fn remember(&self, error: CallError) {
        let mut stored = self.import_error.borrow_mut();
        if stored.is_none() {
            *stored = Some(error);
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
            Ok(result) => lower_result_tracked(&result, signature, &self.resources),
            Err(error) => {
                self.remember(error);
                default_result(signature)
            }
        }
    }

    async fn drop_resource(&self, interface: String, name: String, id: u32) -> Result<(), JsValue> {
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
