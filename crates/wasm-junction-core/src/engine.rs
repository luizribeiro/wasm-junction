use std::collections::BTreeMap;
use std::sync::Arc;
use std::{error::Error, fmt};

use crate::{BoxFuture, CallError, HostBound, InvocationContext, InvocationId, Resource, Vals};

/// Name shared by applications and engines for the built-in WASI provider.
pub const WASI_PROVIDER_NAME: &str = "WASI";

/// Name shared by applications and engines for the built-in WASI HTTP provider.
pub const WASI_HTTP_PROVIDER_NAME: &str = "WASI HTTP";

/// Engine-neutral WASI settings for one component.
///
/// Nothing from the host environment is visible unless it is added explicitly.
///
/// ```
/// use wasm_junction_core::WasiSettings;
///
/// let settings = WasiSettings::new().env("MODE", "preview").arg("notes.txt");
/// assert_eq!(settings.environment().collect::<Vec<_>>(), [("MODE", "preview")]);
/// assert_eq!(settings.arguments().collect::<Vec<_>>(), ["notes.txt"]);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WasiSettings {
    environment: BTreeMap<String, String>,
    arguments: Vec<String>,
}

impl WasiSettings {
    /// Creates settings with no environment variables or arguments.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            environment: BTreeMap::new(),
            arguments: Vec::new(),
        }
    }

    /// Makes one environment variable visible, replacing an earlier value for its name.
    #[must_use]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.insert(name.into(), value.into());
        self
    }

    /// Appends one argument visible through `wasi:cli/environment`.
    #[must_use]
    pub fn arg(mut self, argument: impl Into<String>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    /// Returns the configured environment variables in name order.
    #[must_use]
    pub fn environment(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.environment
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// Returns the configured arguments in insertion order.
    #[must_use]
    pub fn arguments(&self) -> impl ExactSizeIterator<Item = &str> {
        self.arguments.iter().map(String::as_str)
    }
}

/// The direction bytes travel across a component boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelDirection {
    /// Bytes are produced by the host and consumed by a guest.
    HostToGuest,
    /// Bytes are produced by a guest and consumed by the host.
    GuestToHost,
}

/// An engine lifecycle event forwarded to application middleware.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineEvent {
    /// An engine-provided resource is about to be dropped.
    ResourceDrop {
        /// The invocation that owns the resource.
        invocation: InvocationId,
        /// The resource being dropped.
        resource: Resource,
    },
    /// A byte stream began crossing an engine boundary.
    ChannelOpen {
        /// The invocation whose call opened the channel.
        invocation: InvocationId,
        /// The opaque stream identifier.
        stream: u64,
        /// The direction bytes travel.
        direction: ChannelDirection,
    },
    /// A byte stream stopped crossing an engine boundary.
    ChannelClose {
        /// The invocation whose call opened the channel.
        invocation: InvocationId,
        /// The opaque stream identifier.
        stream: u64,
        /// The direction bytes traveled.
        direction: ChannelDirection,
    },
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

    /// Reports an engine lifecycle event to the application.
    fn emit(&self, _event: EngineEvent) {}
}

/// Compiles WebAssembly components for an application dispatcher.
///
/// Engines live in separate crates so applications choose their runtime explicitly.
pub trait Engine: HostBound {
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
