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
    JsResult, lift_args, lift_result, lift_result_error, lower_args, lower_result,
};
use crate::{TranspiledComponent, transpile_component};

#[wasm_bindgen(module = "/js/adapter.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = compileComponent)]
    async fn compile_component(
        source: &str,
        names: &Array,
        modules: &Array,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch)]
    async fn invoke(
        runtime: &JsValue,
        interface: &str,
        function: &str,
        args: &Array,
        dispatch: &js_sys::Function,
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
    let runtime = compile_component(&plan.source, &names, &modules)
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
        let args = lower_args(args, &signature);
        Box::pin(async move {
            let args = args?;
            let import_error = Rc::new(RefCell::new(None));
            let bridge = Bridge {
                imports,
                context,
                component,
                signatures: self.signatures.clone(),
                import_error: import_error.clone(),
            };
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
            self.instantiations
                .set(self.instantiations.get().saturating_add(1));
            let result = invoke(
                &self.runtime,
                &interface,
                &function,
                &args,
                callback.as_ref().unchecked_ref(),
            )
            .await;
            match result {
                Ok(result) => lift_result(result, &signature),
                Err(error) => {
                    if let Some(error) = import_error.borrow_mut().take() {
                        return Err(error);
                    }
                    if let Some(result) = lift_result_error(&error, &signature)? {
                        return Ok(result);
                    }
                    Err(CallError::trap(format!(
                        "component export `{interface}#{function}` trapped: {}",
                        js_error(&error)
                    )))
                }
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
        let args = lift_args(args, signature)?;
        let result = self
            .imports
            .call(
                self.context.clone(),
                self.component.clone(),
                Arc::from(resolved_interface),
                Arc::from(function),
                args,
            )
            .await?;
        lower_result(&result, signature)
    }
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
