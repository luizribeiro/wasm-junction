use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use js_sys::{Array, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_junction_core::{
    BoxFuture, CallError, CompiledComponent, Engine, EngineError, ImportDispatcher,
    InvocationContext, Vals, WasiConfig,
};

use crate::types::Signatures;
use crate::{TranspiledComponent, transpile_component};

#[wasm_bindgen(module = "/js/adapter.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = compileComponent)]
    async fn compile_component(
        source: &str,
        names: &Array,
        modules: &Array,
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
        let plan = transpile_component(&bytes).map_err(EngineError::new);
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
        _instantiations: instantiations,
    }))
}

struct BrowserCompiled {
    runtime: JsValue,
    signatures: Signatures,
    _instantiations: Rc<Cell<u64>>,
}

impl CompiledComponent for BrowserCompiled {
    fn call(
        &self,
        _imports: Arc<dyn ImportDispatcher>,
        _context: InvocationContext,
        _component: Arc<str>,
        _interface: Arc<str>,
        _function: Arc<str>,
        _args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        let _ = (&self.runtime, &self.signatures);
        Box::pin(std::future::ready(Err(CallError::unavailable(
            "jco export calls are not initialized",
        ))))
    }
}

fn js_error(value: &JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("JavaScript error: {value:?}"))
}
