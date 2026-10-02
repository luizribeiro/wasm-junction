use std::collections::BTreeMap;
use std::sync::Arc;
use std::{error::Error, fmt};

use crate::{BoxFuture, CallError, HostBound, InvocationContext, Resource, Vals};

/// Name shared by applications and engines for the built-in WASI provider.
pub const WASI_PROVIDER_NAME: &str = "WASI";

/// The direction bytes travel across a component boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelDirection {
    /// Bytes are produced by the host and consumed by a guest.
    HostToGuest,
    /// Bytes are produced by a guest and consumed by the host.
    GuestToHost,
}

/// The engine-provided implementation at the end of an imported call's middleware chain.
pub trait ImportTarget: HostBound {
    /// Invokes the implementation with middleware's final arguments.
    fn call(
        &self,
        context: InvocationContext,
        args: Vals,
    ) -> BoxFuture<'static, Result<Vals, CallError>>;
}

/// A compiled component whose exports can be invoked by an application dispatcher.
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
    ) -> BoxFuture<'_, Result<Vals, CallError>>;
}

/// The engine-facing route from a guest import back into an application dispatcher.
pub trait ImportDispatcher: HostBound {
    /// Invokes one imported function on behalf of `caller`.
    fn call(
        &self,
        context: InvocationContext,
        caller: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>>;

    /// Invokes an engine-provided import through the application middleware chain.
    fn call_engine(
        &self,
        context: InvocationContext,
        caller: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
        target: Arc<dyn ImportTarget>,
    ) -> BoxFuture<'_, Result<Vals, CallError>>;

    /// Drops a host resource owned by `caller` without passing through call middleware.
    fn drop_resource(
        &self,
        context: InvocationContext,
        caller: Arc<str>,
        resource: Resource,
    ) -> BoxFuture<'_, Result<(), CallError>>;

    /// Reports that an engine began bridging a stream across its boundary.
    fn channel_open(&self, _stream: u64, _direction: ChannelDirection) {}

    /// Reports that an engine finished bridging a stream across its boundary.
    fn channel_close(&self, _stream: u64, _direction: ChannelDirection) {}
}

/// Compiles WebAssembly components for an application dispatcher.
///
/// Engines live in separate crates so applications choose their runtime explicitly.
pub trait Engine: HostBound {
    /// Reports whether this engine supplies an imported interface directly.
    fn supports_import(&self, _interface: &str) -> bool {
        false
    }

    /// Returns the interfaces supplied by a named engine-provided provider.
    ///
    /// Engine and facade crates must use a shared provider-name constant, such as
    /// [`WASI_PROVIDER_NAME`]. The application matches the returned versioned interface names
    /// semver-compatibly against guest imports. Return `None` when the engine does not implement
    /// the named provider so registration fails during application construction.
    fn provider_interfaces(&self, _provider: &str) -> Option<&'static [&'static str]> {
        None
    }

    /// Compiles component bytes into a reusable execution plan.
    fn compile(
        &self,
        bytes: Arc<[u8]>,
        wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>>;
}

/// Engine-neutral WASI settings applied to every component invocation.
///
/// No host environment variables are visible unless they are added explicitly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WasiConfig {
    environment: BTreeMap<String, String>,
}

impl WasiConfig {
    /// Creates a configuration with no environment variables.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            environment: BTreeMap::new(),
        }
    }

    /// Makes one environment variable visible, replacing an earlier value for its name.
    #[must_use]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.insert(name.into(), value.into());
        self
    }

    /// Returns the configured environment variables in name order.
    #[must_use]
    pub fn environment(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.environment
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
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
