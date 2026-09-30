use std::sync::Arc;
use std::{error::Error, fmt};

use crate::{BoxFuture, HostBound, InvocationContext, Trap, Vals};

/// A compiled component whose exports can be invoked by an [`crate::App`].
///
/// Implement this in an engine crate. Each call must run in a fresh component instance.
pub trait CompiledComponent: HostBound {
    /// Invokes one exported function, routing its imports through `imports`.
    fn call(
        &self,
        imports: Arc<dyn ImportDispatcher>,
        context: InvocationContext,
        component: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
    ) -> BoxFuture<'_, Result<Vals, Trap>>;
}

/// The engine-facing route from a guest import back into an [`crate::App`].
pub trait ImportDispatcher: HostBound {
    /// Invokes one imported function on behalf of `caller`.
    fn call(
        &self,
        context: InvocationContext,
        caller: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
    ) -> BoxFuture<'_, Result<Vals, Trap>>;
}

/// Compiles WebAssembly components for an [`crate::App`].
///
/// Engines live in separate crates so applications choose their runtime explicitly.
pub trait Engine: HostBound {
    /// Compiles component bytes into a reusable execution plan.
    fn compile(
        &self,
        bytes: Arc<[u8]>,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>>;
}

/// A failure to compile component bytes for an engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineError(String);

impl EngineError {
    /// Creates a compilation error with an engine-provided diagnostic.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for EngineError {}
