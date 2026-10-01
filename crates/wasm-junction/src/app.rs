use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt::{self, Display};
use std::panic::Location;
use std::sync::{Arc, Mutex, MutexGuard};

use semver::Version;

use crate::middleware::{CallTarget, ErasedMiddleware};
use crate::{
    BoxFuture, Call, CallContext, CallError, Caller, CompiledComponent, Component, Engine,
    EngineError, Event, ImportDispatcher, ImportTarget, InvocationContext, Middleware, Provided,
    Provider, Val, Vals, WasiConfig,
};

/// An application assembled from host providers, middleware, and WebAssembly components.
#[derive(Clone)]
pub struct App(pub(crate) Arc<AppInner>);

pub(crate) struct AppInner {
    engine: Arc<dyn Engine>,
    providers: HashMap<&'static str, Arc<dyn Provider>>,
    #[allow(dead_code, reason = "export dispatch runs the middleware chain")]
    middleware: Arc<[Arc<dyn ErasedMiddleware>]>,
    wasi: WasiConfig,
    components: Mutex<BTreeMap<String, LoadedComponent>>,
}

struct LoadedComponent {
    name: Arc<str>,
    exports: Vec<Arc<str>>,
    links: HashMap<String, String>,
    compiled: Arc<dyn CompiledComponent>,
}

impl LoadedComponent {
    fn export_name(&self, interface: &str) -> Option<Arc<str>> {
        self.exports
            .iter()
            .find(|export| interfaces_compatible(interface, export))
            .cloned()
    }

    fn exports_interface(&self, interface: &str) -> bool {
        self.export_name(interface).is_some()
    }
}

impl App {
    /// Starts configuring an application.
    #[must_use]
    pub fn builder() -> AppBuilder {
        AppBuilder::default()
    }

    /// Compiles and loads a named component.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] if the component is unnamed, its name is already loaded, an import
    /// is missing, or the engine cannot compile it.
    pub async fn load(&self, component: Component) -> Result<(), LoadError> {
        let (bytes, name, imports, exports) = component.into_parts();
        let name = name.ok_or(LoadError::UnnamedComponent)?;
        if self.lock_components().contains_key(&name) {
            return Err(LoadError::DuplicateName(name));
        }
        let mut missing = imports
            .iter()
            .filter(|import| {
                matches!(
                    self.resolve_import(&name, import),
                    Err(ResolveError::Missing)
                ) && !self.0.engine.supports_import(import)
            })
            .cloned()
            .collect::<Vec<_>>();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            return Err(LoadError::MissingImports(MissingImports::new(missing)));
        }
        let compiled = self
            .0
            .engine
            .compile(bytes, self.0.wasi.clone())
            .await
            .map_err(LoadError::Compile)?;
        let mut components = self.lock_components();
        if components.contains_key(&name) {
            return Err(LoadError::DuplicateName(name));
        }
        components.insert(
            name.clone(),
            LoadedComponent {
                name: Arc::from(name),
                exports: exports.into_iter().map(Arc::from).collect(),
                links: HashMap::new(),
                compiled,
            },
        );
        Ok(())
    }

    /// Returns a generated handle for one component interface.
    ///
    /// # Errors
    ///
    /// Returns [`GetError`] if the component is unknown or does not export the interface.
    pub fn get<I: InterfaceHandle>(&self, name: &str) -> Result<I, GetError> {
        let components = self.lock_components();
        let component = components
            .get(name)
            .ok_or_else(|| GetError::UnknownComponent(name.to_owned()))?;
        if !component.exports_interface(I::INTERFACE) {
            return Err(GetError::MissingExport {
                component: name.to_owned(),
                interface: I::INTERFACE,
            });
        }
        Ok(I::from_app(self.clone(), component.name.clone()))
    }

    /// Returns generated handles for every component exporting `I`.
    #[must_use]
    pub fn all<I: InterfaceHandle>(&self) -> Vec<(String, I)> {
        self.lock_components()
            .iter()
            .filter(|(_, component)| component.exports_interface(I::INTERFACE))
            .map(|(name, component)| {
                (
                    name.clone(),
                    I::from_app(self.clone(), component.name.clone()),
                )
            })
            .collect()
    }

    /// Reports whether a named component exports `I`.
    #[must_use]
    pub fn has<I: InterfaceHandle>(&self, name: &str) -> bool {
        self.lock_components()
            .get(name)
            .is_some_and(|component| component.exports_interface(I::INTERFACE))
    }

    /// Directs one component import to a named component provider.
    ///
    /// # Errors
    ///
    /// Returns [`LinkError`] when either component is unknown or the provider does not export a
    /// compatible interface.
    pub fn link(
        &self,
        consumer: &str,
        interface: impl Into<String>,
        provider: &str,
    ) -> Result<(), LinkError> {
        let interface = interface.into();
        let mut components = self.lock_components();
        let target = components
            .get(provider)
            .ok_or_else(|| LinkError::UnknownComponent(provider.to_owned()))?;
        if !target.exports_interface(&interface) {
            return Err(LinkError::MissingExport {
                component: provider.to_owned(),
                interface,
            });
        }
        components
            .get_mut(consumer)
            .ok_or_else(|| LinkError::UnknownComponent(consumer.to_owned()))?
            .links
            .insert(interface, provider.to_owned());
        Ok(())
    }

    /// Calls an exported function through the application dispatcher.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] if the component or export is unavailable, middleware refuses the call,
    /// or the engine reports a failure.
    pub async fn call(
        &self,
        component: &str,
        interface: &str,
        function: impl Into<Arc<str>>,
        args: Vals,
    ) -> Result<Vals, CallError> {
        self.call_with_context(
            component,
            interface,
            function.into(),
            args,
            InvocationContext::default(),
        )
        .await
    }

    async fn call_with_context(
        &self,
        component: &str,
        interface: &str,
        function: Arc<str>,
        args: Vals,
        context: InvocationContext,
    ) -> Result<Vals, CallError> {
        let (compiled, component_name, interface) = {
            let components = self.lock_components();
            let loaded = components.get(component).ok_or_else(|| {
                CallError::unavailable(format!("component `{component}` is not loaded"))
            })?;
            let resolved = loaded.export_name(interface).ok_or_else(|| {
                CallError::unavailable(format!(
                    "component `{component}` does not export `{interface}`"
                ))
            })?;
            (loaded.compiled.clone(), loaded.name.clone(), resolved)
        };
        self.dispatch(
            Arc::new(ComponentTarget {
                compiled,
                imports: Arc::new(self.clone()),
                context,
                component: component_name.clone(),
                component_boundary: false,
            }),
            Call::new(Caller::Host, component_name, interface, function, args),
        )
        .await
    }

    async fn call_import(
        &self,
        context: InvocationContext,
        caller: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
    ) -> Result<Vals, CallError> {
        let (destination, resolved_interface, target): (_, _, Arc<dyn CallTarget>) =
            match self.resolve_import(&caller, &interface) {
                Ok(ResolvedImport::Host {
                    interface,
                    provider,
                }) => (
                    Arc::from("host"),
                    Arc::from(interface),
                    Arc::new(HostTarget { provider, context }),
                ),
                Ok(ResolvedImport::Component {
                    name,
                    interface,
                    compiled,
                }) => (
                    name.clone(),
                    interface,
                    Arc::new(ComponentTarget {
                        compiled,
                        imports: Arc::new(self.clone()),
                        context,
                        component: name,
                        component_boundary: true,
                    }),
                ),
                Err(ResolveError::Missing) => {
                    return Err(CallError::unavailable(format!(
                        "no provider for `{interface}`"
                    )));
                }
                Err(ResolveError::Ambiguous) => {
                    return Err(CallError::refused(format!(
                        "more than one provider for `{interface}`"
                    )));
                }
            };
        self.dispatch(
            target,
            Call::new(
                Caller::Component(caller),
                destination,
                resolved_interface,
                function,
                args,
            ),
        )
        .await
    }

    async fn call_engine_import(
        &self,
        context: InvocationContext,
        caller: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
        target: Arc<dyn ImportTarget>,
    ) -> Result<Vals, CallError> {
        self.dispatch(
            Arc::new(EngineTarget { target, context }),
            Call::new(Caller::Component(caller), "host", interface, function, args),
        )
        .await
    }

    async fn dispatch(&self, target: Arc<dyn CallTarget>, call: Call) -> Result<Vals, CallError> {
        self.emit(&Event::InvocationStart {
            component: call.callee.clone(),
        });
        let result = crate::Next::new(self.0.middleware.clone(), target)
            .run(call.clone())
            .await;
        self.emit(&Event::InvocationEnd {
            component: call.callee,
        });
        result
    }

    fn emit(&self, event: &Event) {
        for middleware in self.0.middleware.iter() {
            middleware.event(event);
        }
    }

    fn resolve_import(
        &self,
        caller: &str,
        requested: &str,
    ) -> Result<ResolvedImport, ResolveError> {
        let components = self.lock_components();
        if let Some(provider_name) = components
            .get(caller)
            .and_then(|component| component.links.get(requested))
        {
            let component = &components[provider_name];
            return Ok(ResolvedImport::Component {
                name: component.name.clone(),
                interface: component
                    .export_name(requested)
                    .ok_or(ResolveError::Missing)?,
                compiled: component.compiled.clone(),
            });
        }
        let hosts = self
            .0
            .providers
            .iter()
            .filter(|(provided, _)| interfaces_compatible(requested, provided))
            .map(|(interface, provider)| ResolvedImport::Host {
                interface,
                provider: provider.clone(),
            });
        let component_candidates = components
            .values()
            .filter(|component| component.name.as_ref() != caller)
            .filter_map(|component| {
                Some(ResolvedImport::Component {
                    name: component.name.clone(),
                    interface: component.export_name(requested)?,
                    compiled: component.compiled.clone(),
                })
            })
            .collect::<Vec<_>>();
        let mut candidates = hosts.chain(component_candidates);
        let candidate = candidates.next().ok_or(ResolveError::Missing)?;
        if candidates.next().is_some() {
            Err(ResolveError::Ambiguous)
        } else {
            Ok(candidate)
        }
    }

    fn lock_components(&self) -> MutexGuard<'_, BTreeMap<String, LoadedComponent>> {
        match self.0.components.lock() {
            Ok(components) => components,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

enum ResolveError {
    Missing,
    Ambiguous,
}

enum ResolvedImport {
    Host {
        interface: &'static str,
        provider: Arc<dyn Provider>,
    },
    Component {
        name: Arc<str>,
        interface: Arc<str>,
        compiled: Arc<dyn CompiledComponent>,
    },
}

impl ImportDispatcher for App {
    fn call(
        &self,
        context: InvocationContext,
        caller: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(self.call_import(context, caller, interface, function, args))
    }

    fn call_engine(
        &self,
        context: InvocationContext,
        caller: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
        target: Arc<dyn ImportTarget>,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(self.call_engine_import(context, caller, interface, function, args, target))
    }
}

struct HostTarget {
    provider: Arc<dyn Provider>,
    context: InvocationContext,
}

impl CallTarget for HostTarget {
    fn call(&self, call: Call) -> BoxFuture<'static, Result<Vals, CallError>> {
        let provider = self.provider.clone();
        let invocation = self.context.clone();
        Box::pin(async move {
            let context = CallContext::new(call.caller.clone(), invocation);
            provider.call(&context, call).await
        })
    }
}

struct ComponentTarget {
    compiled: Arc<dyn CompiledComponent>,
    imports: Arc<dyn ImportDispatcher>,
    context: InvocationContext,
    component: Arc<str>,
    component_boundary: bool,
}

struct EngineTarget {
    target: Arc<dyn ImportTarget>,
    context: InvocationContext,
}

impl CallTarget for EngineTarget {
    fn call(&self, call: Call) -> BoxFuture<'static, Result<Vals, CallError>> {
        self.target.call(self.context.clone(), call.args)
    }
}

impl CallTarget for ComponentTarget {
    fn call(&self, call: Call) -> BoxFuture<'static, Result<Vals, CallError>> {
        let compiled = self.compiled.clone();
        let imports = self.imports.clone();
        let context = self.context.clone();
        let component = self.component.clone();
        let component_boundary = self.component_boundary;
        Box::pin(async move {
            if component_boundary && !values_are_plain(&call.args) {
                return Err(CallError::refused(
                    "only plain values can cross between components",
                ));
            }
            let result = compiled
                .call(
                    imports,
                    context,
                    component,
                    call.interface,
                    call.function,
                    call.args,
                )
                .await?;
            if component_boundary && !values_are_plain(&result) {
                return Err(CallError::refused(
                    "only plain values can cross between components",
                ));
            }
            Ok(result)
        })
    }
}

fn values_are_plain(values: &[Val]) -> bool {
    values.iter().all(|value| match value {
        Val::List(values) | Val::Tuple(values) => values_are_plain(values),
        Val::Record(fields) => fields
            .iter()
            .all(|(_, value)| values_are_plain(std::slice::from_ref(value))),
        Val::Variant { value, .. } | Val::Option(value) => value
            .as_deref()
            .is_none_or(|value| values_are_plain(std::slice::from_ref(value))),
        Val::Result(result) => result
            .as_ref()
            .map_or_else(|error| error.as_deref(), |ok| ok.as_deref())
            .is_none_or(|value| values_are_plain(std::slice::from_ref(value))),
        Val::Bool(_)
        | Val::S8(_)
        | Val::U8(_)
        | Val::S16(_)
        | Val::U16(_)
        | Val::S32(_)
        | Val::U32(_)
        | Val::S64(_)
        | Val::U64(_)
        | Val::F32(_)
        | Val::F64(_)
        | Val::Char(_)
        | Val::String(_)
        | Val::Enum(_)
        | Val::Flags(_) => true,
        _ => false,
    })
}

/// The construction contract implemented by each generated interface handle.
pub trait InterfaceHandle: Sized {
    /// The fully qualified WIT interface name.
    const INTERFACE: &'static str;

    /// Creates a handle that routes calls to `component` through `app`.
    fn from_app(app: App, component: Arc<str>) -> Self;
}

/// Missing host interfaces found while loading a component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingImports {
    interfaces: Vec<String>,
}

impl MissingImports {
    fn new(interfaces: Vec<String>) -> Self {
        Self { interfaces }
    }

    /// Returns the missing versioned interface names.
    #[must_use]
    pub fn interfaces(&self) -> &[String] {
        &self.interfaces
    }
}

impl Display for MissingImports {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "missing imports: {}", self.interfaces.join(", "))
    }
}

impl Error for MissingImports {}

/// A failure to load a component.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// Components created from bytes must be named before loading.
    UnnamedComponent,
    /// Another component is already loaded under this name.
    DuplicateName(String),
    /// One or more imported interfaces have no host provider.
    MissingImports(MissingImports),
    /// The selected engine could not compile the component.
    Compile(EngineError),
}

impl Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnnamedComponent => formatter.write_str("component has no application name"),
            Self::DuplicateName(name) => write!(formatter, "component `{name}` is already loaded"),
            Self::MissingImports(error) => Display::fmt(error, formatter),
            Self::Compile(error) => write!(formatter, "component compilation failed: {error}"),
        }
    }
}

impl Error for LoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingImports(error) => Some(error),
            Self::Compile(error) => Some(error),
            Self::UnnamedComponent | Self::DuplicateName(_) => None,
        }
    }
}

/// A failure to obtain a typed interface handle.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GetError {
    /// No component is loaded under this name.
    UnknownComponent(String),
    /// The named component does not export the requested interface.
    MissingExport {
        /// The application component name.
        component: String,
        /// The requested interface.
        interface: &'static str,
    },
}

impl Display for GetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownComponent(name) => write!(formatter, "component `{name}` is not loaded"),
            Self::MissingExport {
                component,
                interface,
            } => write!(
                formatter,
                "component `{component}` does not export `{interface}`"
            ),
        }
    }
}

impl Error for GetError {}

/// A failure to link one component import to another component.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// No component is loaded under the given name.
    UnknownComponent(String),
    /// The selected provider does not export a compatible interface.
    MissingExport {
        /// The provider component name.
        component: String,
        /// The requested interface.
        interface: String,
    },
}

impl Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownComponent(name) => write!(formatter, "component `{name}` is not loaded"),
            Self::MissingExport {
                component,
                interface,
            } => write!(
                formatter,
                "component `{component}` does not export `{interface}`"
            ),
        }
    }
}

impl Error for LinkError {}

struct ProviderRegistration {
    provided: Provided,
    location: &'static Location<'static>,
}

/// Incrementally configures an [`App`].
#[derive(Default)]
pub struct AppBuilder {
    engine: Option<Arc<dyn Engine>>,
    providers: Vec<ProviderRegistration>,
    middleware: Vec<Arc<dyn ErasedMiddleware>>,
    wasi: WasiConfig,
}

impl AppBuilder {
    /// Selects the engine used to compile subsequently loaded components.
    #[must_use]
    pub fn engine(mut self, engine: impl Engine + 'static) -> Self {
        self.engine = Some(Arc::new(engine));
        self
    }

    /// Registers one host interface provider.
    #[track_caller]
    #[must_use]
    pub fn provide(mut self, provided: Provided) -> Self {
        self.providers.push(ProviderRegistration {
            provided,
            location: Location::caller(),
        });
        self
    }

    /// Appends middleware; the first registered middleware runs outermost.
    #[must_use]
    pub fn middleware(mut self, middleware: impl Middleware + 'static) -> Self {
        self.middleware.push(Arc::new(middleware));
        self
    }

    /// Sets the WASI capabilities available to component invocations.
    #[must_use]
    pub fn wasi(mut self, wasi: WasiConfig) -> Self {
        self.wasi = wasi;
        self
    }

    /// Applies a reusable builder transformation.
    #[must_use]
    pub fn apply(self, transform: impl FnOnce(Self) -> Self) -> Self {
        transform(self)
    }

    /// Validates the configuration and creates an application.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] if no engine is available or an interface has multiple providers.
    pub fn build(self) -> Result<App, BuildError> {
        self.build_with(default_engine)
    }

    fn build_with(
        self,
        default: impl FnOnce() -> Result<Arc<dyn Engine>, BuildError>,
    ) -> Result<App, BuildError> {
        let engine = self.engine.map_or_else(default, Ok)?;
        let mut providers = HashMap::new();
        let mut locations = HashMap::new();
        for registration in self.providers {
            let (interface, provider) = registration.provided.into_parts();
            if let Some(first) = locations.insert(interface, registration.location) {
                return Err(BuildError::DuplicateProvider {
                    interface,
                    first,
                    second: registration.location,
                });
            }
            providers.insert(interface, provider);
        }
        Ok(App(Arc::new(AppInner {
            engine,
            providers,
            middleware: self.middleware.into(),
            wasi: self.wasi,
            components: Mutex::new(BTreeMap::new()),
        })))
    }
}

/// A configuration error found while building an [`App`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// No engine was selected and this target has no enabled default.
    MissingEngine,
    /// The target's default engine could not be initialized.
    DefaultEngine(EngineError),
    /// More than one provider was registered for an interface.
    DuplicateProvider {
        /// The duplicated interface.
        interface: &'static str,
        /// The first registration site.
        first: &'static Location<'static>,
        /// The second registration site.
        second: &'static Location<'static>,
    },
}

impl Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEngine => write!(
                formatter,
                "no default engine is available for target `{}`; call `.engine(…)`",
                env!("WASM_JUNCTION_TARGET")
            ),
            Self::DefaultEngine(error) => write!(formatter, "default engine failed: {error}"),
            Self::DuplicateProvider {
                interface,
                first,
                second,
            } => write!(
                formatter,
                "provider for `{interface}` registered at {first} and again at {second}"
            ),
        }
    }
}

impl Error for BuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DefaultEngine(error) => Some(error),
            Self::MissingEngine | Self::DuplicateProvider { .. } => None,
        }
    }
}

#[cfg(all(feature = "wasmtime", not(target_family = "wasm")))]
fn default_engine() -> Result<Arc<dyn Engine>, BuildError> {
    wasm_junction_wasmtime::WasmtimeEngine::new()
        .map(|engine| Arc::new(engine) as Arc<dyn Engine>)
        .map_err(|error| BuildError::DefaultEngine(EngineError::new(error.to_string())))
}

#[cfg(not(all(feature = "wasmtime", not(target_family = "wasm"))))]
fn default_engine() -> Result<Arc<dyn Engine>, BuildError> {
    Err(BuildError::MissingEngine)
}

fn interfaces_compatible(requested: &str, provided: &str) -> bool {
    if requested == provided {
        return true;
    }
    let (Some((requested_name, requested_version)), Some((provided_name, provided_version))) =
        (requested.rsplit_once('@'), provided.rsplit_once('@'))
    else {
        return false;
    };
    if requested_name != provided_name {
        return false;
    }
    let (Ok(requested), Ok(provided)) = (
        Version::parse(requested_version),
        Version::parse(provided_version),
    ) else {
        return false;
    };
    if requested.major == provided.major
        && requested.minor == provided.minor
        && requested.patch == provided.patch
        && requested.pre == provided.pre
    {
        return true;
    }
    if !requested.pre.is_empty() || !provided.pre.is_empty() {
        return false;
    }
    requested.major == provided.major
        && if requested.major == 0 {
            requested.minor != 0 && requested.minor == provided.minor
        } else {
            true
        }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use super::*;

    struct ExplicitEngine;

    struct EchoTarget;

    impl ImportTarget for EchoTarget {
        fn call(
            &self,
            _context: InvocationContext,
            args: Vals,
        ) -> BoxFuture<'static, Result<Vals, CallError>> {
            Box::pin(async move { Ok(args) })
        }
    }

    struct RewriteEngineImport;

    impl Middleware for RewriteEngineImport {
        async fn call(&self, mut call: Call, next: crate::Next) -> Result<Vals, CallError> {
            assert_eq!(call.caller, Caller::Component(Arc::from("guest")));
            assert_eq!(call.interface.as_ref(), "system:settings/config@1.0.0");
            assert_eq!(call.function.as_ref(), "read");
            call.args = vec!["rewritten".into()];
            next.run(call).await
        }
    }

    impl Engine for ExplicitEngine {
        fn compile(
            &self,
            _bytes: Arc<[u8]>,
            _wasi: WasiConfig,
        ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
            Box::pin(async { Err(EngineError::new("unused engine")) })
        }
    }

    #[test]
    fn explicit_engine_skips_the_default_factory() {
        let default_calls = Cell::new(0);
        App::builder()
            .engine(ExplicitEngine)
            .build_with(|| {
                default_calls.set(default_calls.get() + 1);
                Err(BuildError::MissingEngine)
            })
            .unwrap();

        assert_eq!(default_calls.get(), 0);
    }

    #[test]
    fn engine_import_targets_run_through_middleware() {
        let app = App::builder()
            .engine(ExplicitEngine)
            .middleware(RewriteEngineImport)
            .build()
            .unwrap();
        let values = ready(app.call_engine(
            InvocationContext::default(),
            Arc::from("guest"),
            Arc::from("system:settings/config@1.0.0"),
            Arc::from("read"),
            vec!["original".into()],
            Arc::new(EchoTarget),
        ))
        .unwrap();

        assert_eq!(values, [crate::Val::from("rewritten")]);
    }

    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("future unexpectedly suspended"),
        }
    }

    #[test]
    fn interface_compatibility_follows_semver_tracks() {
        let compatible = [
            ("a:b/c", "a:b/c"),
            ("a:b/c@1.2.3", "a:b/c@1.8.0"),
            ("a:b/c@0.4.1", "a:b/c@0.4.9"),
            ("a:b/c@1.2.3+abc", "a:b/c@1.8.0+def"),
            ("a:b/c@0.0.1+abc", "a:b/c@0.0.1+def"),
            ("a:b/c@1.2.3-rc.1+abc", "a:b/c@1.2.3-rc.1+def"),
        ];
        let incompatible = [
            ("a:b/c", "a:b/d"),
            ("a:b/c", "a:b/c@1.0.0"),
            ("a:b/c@1.0.0", "a:b/c@2.0.0"),
            ("a:b/c@0.4.0", "a:b/c@0.5.0"),
            ("a:b/c@0.0.1", "a:b/c@0.0.2"),
            ("a:b/c@1.2.3-rc.1", "a:b/c@1.2.3-rc.2"),
            ("a:b/c@bad", "a:b/c@worse"),
        ];
        for (left, right) in compatible {
            assert!(interfaces_compatible(left, right));
            assert!(interfaces_compatible(right, left));
        }
        for (left, right) in incompatible {
            assert!(!interfaces_compatible(left, right));
            assert!(!interfaces_compatible(right, left));
        }
    }

    #[test]
    fn nested_values_are_plain() {
        let values = vec![Val::Record(vec![(
            String::from("items"),
            Val::List(vec![Val::Option(Some(Box::new(Val::U32(3))))]),
        )])];

        assert!(values_are_plain(&values));
    }
}
