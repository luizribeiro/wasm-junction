use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt::{self, Display};
use std::panic::Location;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::middleware::{CallTarget, ErasedMiddleware};
use crate::{
    BoxFuture, Call, CallContext, CallError, Caller, CompiledComponent, Component, Engine,
    EngineError, Event, ImportDispatcher, InvocationContext, Middleware, Provided, Provider, Vals,
};

/// An application assembled from host providers, middleware, and WebAssembly components.
#[derive(Clone)]
pub struct App(pub(crate) Arc<AppInner>);

pub(crate) struct AppInner {
    engine: Arc<dyn Engine>,
    providers: HashMap<&'static str, Arc<dyn Provider>>,
    #[allow(dead_code, reason = "export dispatch runs the middleware chain")]
    middleware: Arc<[Arc<dyn ErasedMiddleware>]>,
    components: Mutex<BTreeMap<String, LoadedComponent>>,
}

struct LoadedComponent {
    name: Arc<str>,
    exports: Vec<Arc<str>>,
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
            .filter(|import| self.find_provider(import).is_none())
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
            .compile(bytes)
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
        let (provided_interface, provider) = self
            .find_provider(&interface)
            .ok_or_else(|| CallError::unavailable(format!("no provider for `{interface}`")))?;
        self.dispatch(
            Arc::new(HostTarget { provider, context }),
            Call::new(
                Caller::Component(caller),
                "host",
                provided_interface,
                function,
                args,
            ),
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

    fn find_provider(&self, requested: &str) -> Option<(&'static str, Arc<dyn Provider>)> {
        self.0
            .providers
            .iter()
            .find(|(provided, _)| interfaces_compatible(requested, provided))
            .map(|(interface, provider)| (*interface, provider.clone()))
    }

    fn lock_components(&self) -> MutexGuard<'_, BTreeMap<String, LoadedComponent>> {
        match self.0.components.lock() {
            Ok(components) => components,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
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
}

impl CallTarget for ComponentTarget {
    fn call(&self, call: Call) -> BoxFuture<'static, Result<Vals, CallError>> {
        let compiled = self.compiled.clone();
        let imports = self.imports.clone();
        let context = self.context.clone();
        let component = self.component.clone();
        Box::pin(async move {
            compiled
                .call(
                    imports,
                    context,
                    component,
                    call.interface,
                    call.function,
                    call.args,
                )
                .await
        })
    }
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

    /// Applies a reusable builder transformation.
    #[must_use]
    pub fn apply(self, transform: impl FnOnce(Self) -> Self) -> Self {
        transform(self)
    }

    /// Validates the configuration and creates an application.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] if no engine was selected or an interface has multiple providers.
    pub fn build(self) -> Result<App, BuildError> {
        let engine = self.engine.ok_or(BuildError::MissingEngine)?;
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
            components: Mutex::new(BTreeMap::new()),
        })))
    }
}

/// A configuration error found while building an [`App`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// No engine was selected.
    MissingEngine,
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
            Self::MissingEngine => formatter.write_str("an engine is required"),
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

impl Error for BuildError {}

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
    let parse = |version: &str| {
        let mut pieces = version.split('.');
        let version = (
            pieces.next()?.parse::<u64>().ok()?,
            pieces.next()?.parse::<u64>().ok()?,
            pieces.next()?.parse::<u64>().ok()?,
        );
        pieces.next().is_none().then_some(version)
    };
    let (Some(requested), Some(provided)) = (parse(requested_version), parse(provided_version))
    else {
        return false;
    };
    requested.0 == provided.0 && (requested.0 != 0 || requested.1 == provided.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_compatibility_follows_semver_tracks() {
        assert!(interfaces_compatible("a:b/c@1.2.3", "a:b/c@1.2.3"));
        assert!(interfaces_compatible("a:b/c@1.2.3", "a:b/c@1.8.0"));
        assert!(interfaces_compatible("a:b/c@1.8.0", "a:b/c@1.2.3"));
        assert!(!interfaces_compatible("a:b/c@1.0.0", "a:b/c@2.0.0"));
        assert!(!interfaces_compatible("a:b/c@0.4.0", "a:b/c@0.5.0"));
        assert!(!interfaces_compatible("a:b/c@bad", "a:b/c@worse"));
    }
}
