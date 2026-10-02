use std::sync::Arc;

use wasm_junction_core::{
    BoxFuture, CallError, CompiledComponent, Engine, EngineError, ImportDispatcher,
    InvocationContext, Vals,
};

use crate::{TranspiledComponent, transpile_component};

/// A jco engine that executes components when compiled to WebAssembly for a browser.
#[derive(Clone, Copy, Debug, Default)]
pub struct JcoEngine;

impl JcoEngine {
    /// Creates a browser component engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns the number of fresh instances created for export calls.
    #[must_use]
    pub const fn instantiations(&self) -> u64 {
        0
    }
}

impl Engine for JcoEngine {
    fn compile(
        &self,
        bytes: Arc<[u8]>,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        let result = transpile_component(&bytes).map(|plan| {
            let _ = (&plan.source, &plan.modules);
            #[cfg(test)]
            let _ = &plan.signatures;
            Arc::new(NativeCompiled { _plan: plan }) as Arc<dyn CompiledComponent>
        });
        Box::pin(std::future::ready(result))
    }
}

struct NativeCompiled {
    _plan: TranspiledComponent,
}

impl CompiledComponent for NativeCompiled {
    fn call(
        &self,
        _imports: Arc<dyn ImportDispatcher>,
        _context: InvocationContext,
        _component: Arc<str>,
        _interface: Arc<str>,
        _function: Arc<str>,
        _args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(std::future::ready(Err(CallError::unavailable(
            "the jco engine executes only on WebAssembly browser targets",
        ))))
    }
}
