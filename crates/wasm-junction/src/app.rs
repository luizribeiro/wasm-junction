use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fmt::{self, Display};
use std::panic::Location;
use std::sync::{Arc, Mutex, MutexGuard};

use semver::Version;

use crate::component::ComponentParts;
use crate::middleware::{CallTarget, ErasedMiddleware};
use crate::{
    BoxFuture, Call, CallContext, CallError, Caller, CompiledComponent, Component, Engine,
    EngineError, Event, HostBound, ImportDispatcher, ImportTarget, InvocationContext, Middleware,
    Provided, Provider, Resource, Val, Vals, WasiConfig,
};

mod lifecycle;

/// An application assembled from host providers, middleware, and WebAssembly components.
#[derive(Clone)]
pub struct App(pub(crate) Arc<AppInner>);

pub(crate) struct AppInner {
    engine: Arc<dyn Engine>,
    providers: HashMap<&'static str, Arc<dyn Provider>>,
    #[allow(dead_code, reason = "export dispatch runs the middleware chain")]
    middleware: Arc<[Arc<dyn ErasedMiddleware>]>,
    wasi: WasiConfig,
    max_call_depth: usize,
    components: Mutex<BTreeMap<String, LoadedComponent>>,
    handle_counts: Mutex<HashMap<(String, &'static str), usize>>,
}

struct LoadedComponent {
    name: Arc<str>,
    generation: Arc<Generation>,
    links: HashMap<String, String>,
}

struct Generation {
    imports: Vec<Arc<str>>,
    exports: Vec<Arc<str>>,
    compiled: Arc<dyn CompiledComponent>,
}

struct PendingComponent {
    bytes: Arc<[u8]>,
    name: String,
    imports: Vec<String>,
    exports: Vec<String>,
}

impl LoadedComponent {
    fn export_name(&self, interface: &str) -> Option<Arc<str>> {
        self.generation
            .exports
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
        let ComponentParts {
            bytes,
            name,
            imports,
            exports,
            resource_exports,
        } = component.into_parts();
        let name = name.ok_or(LoadError::UnnamedComponent)?;
        reject_resource_exports(resource_exports)?;
        self.validate_load(&name, &imports, &exports, &self.lock_components())?;
        let compiled = self
            .0
            .engine
            .compile(bytes, self.0.wasi.clone())
            .await
            .map_err(LoadError::Compile)?;
        let mut components = self.lock_components();
        self.validate_load(&name, &imports, &exports, &components)?;
        components.insert(
            name.clone(),
            LoadedComponent {
                name: Arc::from(name),
                generation: Arc::new(Generation {
                    imports: imports.into_iter().map(Arc::from).collect(),
                    exports: exports.into_iter().map(Arc::from).collect(),
                    compiled,
                }),
                links: HashMap::new(),
            },
        );
        Ok(())
    }

    /// Compiles and loads a set of components as one atomic operation.
    /// Imports may resolve to any component in the set, regardless of iteration order.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] without loading any component when a name is invalid or duplicated,
    /// resolution would fail or become ambiguous, or an engine compilation fails.
    pub async fn load_all(
        &self,
        components: impl IntoIterator<Item = Component>,
    ) -> Result<(), LoadError> {
        let mut pending = Vec::new();
        for component in components {
            let ComponentParts {
                bytes,
                name,
                imports,
                exports,
                resource_exports,
            } = component.into_parts();
            reject_resource_exports(resource_exports)?;
            pending.push(PendingComponent {
                bytes,
                name: name.ok_or(LoadError::UnnamedComponent)?,
                imports,
                exports,
            });
        }
        self.validate_batch(&pending, &self.lock_components())?;
        let mut compiled = Vec::with_capacity(pending.len());
        for component in &pending {
            compiled.push(
                self.0
                    .engine
                    .compile(component.bytes.clone(), self.0.wasi.clone())
                    .await
                    .map_err(LoadError::Compile)?,
            );
        }
        let mut loaded = self.lock_components();
        self.validate_batch(&pending, &loaded)?;
        for (component, compiled) in pending.into_iter().zip(compiled) {
            loaded.insert(
                component.name.clone(),
                LoadedComponent {
                    name: Arc::from(component.name),
                    generation: Arc::new(Generation {
                        imports: component.imports.into_iter().map(Arc::from).collect(),
                        exports: component.exports.into_iter().map(Arc::from).collect(),
                        compiled,
                    }),
                    links: HashMap::new(),
                },
            );
        }
        Ok(())
    }

    fn validate_batch(
        &self,
        pending: &[PendingComponent],
        loaded: &BTreeMap<String, LoadedComponent>,
    ) -> Result<(), LoadError> {
        let mut names = HashSet::new();
        for component in pending {
            if loaded.contains_key(&component.name) || !names.insert(component.name.as_str()) {
                return Err(LoadError::DuplicateName(component.name.clone()));
            }
        }
        let added_candidates = |interface: &str, excluded: Option<&str>| {
            pending
                .iter()
                .filter(|component| excluded != Some(component.name.as_str()))
                .filter(|component| {
                    component
                        .exports
                        .iter()
                        .any(|export| interfaces_compatible(interface, export))
                })
                .map(|component| Candidate::Component(component.name.clone()))
                .collect::<Vec<_>>()
        };
        let mut missing = Vec::new();
        let mut issues = Vec::new();
        for component in pending {
            for import in &component.imports {
                if self.0.engine.supports_import(import) {
                    continue;
                }
                let mut candidates = resolution_candidates_excluding(
                    &self.0.providers,
                    loaded,
                    import,
                    Some(&component.name),
                );
                candidates.extend(added_candidates(import, Some(&component.name)));
                match candidates.len() {
                    0 => missing.push(import.clone()),
                    1 => {}
                    _ => issues.push(ResolutionIssue::ambiguous(
                        component.name.clone(),
                        import.clone(),
                        candidates,
                    )),
                }
            }
        }
        for (consumer, component) in loaded {
            for import in &component.generation.imports {
                if component.links.contains_key(import.as_ref()) {
                    continue;
                }
                let mut candidates = resolution_candidates_excluding(
                    &self.0.providers,
                    loaded,
                    import,
                    Some(consumer),
                );
                if candidates.len() == 1 {
                    candidates.extend(added_candidates(import, None));
                }
                if candidates.len() > 1 {
                    issues.push(ResolutionIssue::ambiguous(
                        consumer.clone(),
                        import.to_string(),
                        candidates,
                    ));
                }
            }
        }
        load_resolution_result(missing, issues)
    }

    fn validate_load(
        &self,
        name: &str,
        imports: &[String],
        exports: &[String],
        components: &BTreeMap<String, LoadedComponent>,
    ) -> Result<(), LoadError> {
        if components.contains_key(name) {
            return Err(LoadError::DuplicateName(name.to_owned()));
        }
        let mut missing = Vec::new();
        let mut issues = Vec::new();
        for import in imports {
            if self.0.engine.supports_import(import) {
                continue;
            }
            let candidates =
                resolution_candidates_excluding(&self.0.providers, components, import, Some(name));
            match candidates.len() {
                0 => missing.push(import.clone()),
                1 => {}
                _ => issues.push(ResolutionIssue::ambiguous(
                    name.to_owned(),
                    import.clone(),
                    candidates,
                )),
            }
        }
        for (consumer, component) in components {
            for import in &component.generation.imports {
                if component.links.contains_key(import.as_ref())
                    || !exports
                        .iter()
                        .any(|export| interfaces_compatible(import, export))
                {
                    continue;
                }
                let mut candidates = resolution_candidates_excluding(
                    &self.0.providers,
                    components,
                    import,
                    Some(consumer),
                );
                if candidates.len() == 1 {
                    candidates.push(Candidate::Component(name.to_owned()));
                    issues.push(ResolutionIssue::ambiguous(
                        consumer.clone(),
                        import.to_string(),
                        candidates,
                    ));
                }
            }
        }
        load_resolution_result(missing, issues)
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

    /// Checks every loaded component import for missing or ambiguous providers.
    ///
    /// # Errors
    ///
    /// Returns [`CheckError`] containing every resolution issue found.
    pub fn check(&self) -> Result<(), CheckError> {
        let imports = self
            .lock_components()
            .iter()
            .flat_map(|(name, component)| {
                component
                    .generation
                    .imports
                    .iter()
                    .map(|interface| (name.clone(), interface.clone()))
            })
            .collect::<Vec<_>>();
        let mut issues = imports
            .into_iter()
            .filter_map(|(component, interface)| {
                if self.0.engine.supports_import(&interface) {
                    return None;
                }
                match self.resolve_import(&component, &interface) {
                    Ok(_) => None,
                    Err(ResolveError::Missing) => {
                        Some(ResolutionIssue::missing(component, interface.to_string()))
                    }
                    Err(ResolveError::Ambiguous { candidates }) => Some(
                        ResolutionIssue::ambiguous(component, interface.to_string(), candidates),
                    ),
                }
            })
            .collect::<Vec<_>>();
        issues.sort_by(|left, right| {
            (&left.component, &left.interface).cmp(&(&right.component, &right.interface))
        });
        if issues.is_empty() {
            Ok(())
        } else {
            Err(CheckError { issues })
        }
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
        self.check_call_depth(&context)?;
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
            (
                loaded.generation.compiled.clone(),
                loaded.name.clone(),
                resolved,
            )
        };
        let call = call_for_invocation(
            &context,
            Call::new(
                Caller::Host,
                component_name.clone(),
                interface,
                function,
                args,
            ),
        );
        self.dispatch(
            Arc::new(ComponentTarget {
                compiled,
                imports: Arc::new(self.clone()),
                context,
                component: component_name.clone(),
                component_boundary: false,
            }),
            call,
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
                    Arc::new(HostTarget {
                        provider,
                        context: context.clone(),
                    }),
                ),
                Ok(ResolvedImport::Component {
                    name,
                    interface,
                    compiled,
                }) => {
                    let context = context.descend();
                    self.check_call_depth(&context)?;
                    (
                        name.clone(),
                        interface,
                        Arc::new(ComponentTarget {
                            compiled,
                            imports: Arc::new(self.clone()),
                            context,
                            component: name,
                            component_boundary: true,
                        }),
                    )
                }
                Err(ResolveError::Missing) => {
                    return Err(CallError::unavailable(format!(
                        "no provider for `{interface}`"
                    )));
                }
                Err(ResolveError::Ambiguous { candidates }) => {
                    return Err(CallError::refused(format!(
                        "more than one provider for `{interface}`: {}",
                        display_candidates(&candidates)
                    )));
                }
            };
        let call = call_for_invocation(
            &context,
            Call::new(
                Caller::Component(caller),
                destination,
                resolved_interface,
                function,
                args,
            ),
        );
        self.dispatch(target, call).await
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
        let call = call_for_invocation(
            &context,
            Call::new(Caller::Component(caller), "host", interface, function, args),
        );
        self.dispatch(Arc::new(EngineTarget { target, context }), call)
            .await
    }

    fn drop_host_resource(&self, cx: &CallContext, resource: Resource) -> Result<(), CallError> {
        let provider = self
            .0
            .providers
            .iter()
            .find(|(interface, _)| interfaces_compatible(resource.interface(), interface))
            .map(|(_, provider)| provider)
            .ok_or_else(|| {
                CallError::unavailable(format!(
                    "no host provider for resource interface `{}`",
                    resource.interface()
                ))
            })?;
        self.emit(&Event::ResourceDrop {
            interface: Arc::from(resource.interface()),
            resource: Arc::from(resource.name()),
            id: resource.id(),
        });
        provider.drop_resource(cx, resource)
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

    fn check_call_depth(&self, context: &InvocationContext) -> Result<(), CallError> {
        if context.call_depth() > self.0.max_call_depth {
            Err(CallError::refused(format!(
                "maximum call depth of {} exceeded",
                self.0.max_call_depth
            )))
        } else {
            Ok(())
        }
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
                compiled: component.generation.compiled.clone(),
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
                    compiled: component.generation.compiled.clone(),
                })
            })
            .collect::<Vec<_>>();
        let mut matches = hosts.chain(component_candidates).collect::<Vec<_>>();
        match matches.len() {
            0 => Err(ResolveError::Missing),
            1 => Ok(matches.remove(0)),
            _ => Err(ResolveError::Ambiguous {
                candidates: matches.iter().map(ResolvedImport::candidate).collect(),
            }),
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
    Ambiguous { candidates: Vec<Candidate> },
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

impl ResolvedImport {
    fn candidate(&self) -> Candidate {
        match self {
            Self::Host { .. } => Candidate::Host,
            Self::Component { name, .. } => Candidate::Component(name.to_string()),
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

    fn drop_resource(
        &self,
        context: InvocationContext,
        caller: Arc<str>,
        resource: Resource,
    ) -> BoxFuture<'_, Result<(), CallError>> {
        Box::pin(async move {
            let context = CallContext::new(Caller::Component(caller), context);
            self.drop_host_resource(&context, resource)
        })
    }

    fn channel_open(&self, stream: u64, direction: crate::ChannelDirection) {
        self.emit(&Event::ChannelOpen { stream, direction });
    }

    fn channel_close(&self, stream: u64, direction: crate::ChannelDirection) {
        self.emit(&Event::ChannelClose { stream, direction });
    }
}

struct HostTarget {
    provider: Arc<dyn Provider>,
    context: InvocationContext,
}

impl CallTarget for HostTarget {
    fn call(&self, call: Call) -> BoxFuture<'static, Result<Vals, CallError>> {
        let provider = self.provider.clone();
        let invocation = self.context.with_extensions(call.extensions().clone());
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
        self.target.call(
            self.context.with_extensions(call.extensions().clone()),
            call.args,
        )
    }
}

impl CallTarget for ComponentTarget {
    fn call(&self, call: Call) -> BoxFuture<'static, Result<Vals, CallError>> {
        let compiled = self.compiled.clone();
        let imports = self.imports.clone();
        let context = self.context.with_extensions(call.extensions().clone());
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

fn call_for_invocation(context: &InvocationContext, mut call: Call) -> Call {
    *call.extensions_mut() = context.extensions().clone();
    call
}

/// Shared state and dispatch behavior wrapped by generated interface handles.
#[doc(hidden)]
#[derive(Clone)]
pub struct Handle {
    _lease: Arc<HandleLease>,
    app: App,
    component: Arc<str>,
    context: InvocationContext,
}

impl Handle {
    /// Creates the backing state for a generated interface handle.
    #[doc(hidden)]
    #[must_use]
    pub fn new(app: App, component: Arc<str>, interface: &'static str) -> Self {
        let key = (component.to_string(), interface);
        *lock_or_recover(&app.0.handle_counts)
            .entry(key.clone())
            .or_default() += 1;
        Self {
            _lease: Arc::new(HandleLease {
                app: app.0.clone(),
                key,
            }),
            app,
            component,
            context: InvocationContext::default(),
        }
    }

    /// Returns a copy with one value attached to its calls.
    #[doc(hidden)]
    #[must_use]
    pub fn with<T: Any + HostBound>(&self, value: T) -> Self {
        let mut extensions = self.context.extensions().clone();
        extensions.insert(value);
        Self {
            context: self.context.with_extensions(extensions),
            ..self.clone()
        }
    }

    /// Returns a copy that continues the invocation represented by `context`.
    #[doc(hidden)]
    #[must_use]
    pub fn within(&self, context: &CallContext) -> Self {
        Self {
            context: context.invocation().descend(),
            ..self.clone()
        }
    }

    /// Calls an exported function with the state carried by this handle.
    #[doc(hidden)]
    pub async fn call(
        &self,
        interface: &str,
        function: &'static str,
        args: Vals,
    ) -> Result<Vals, CallError> {
        self.app
            .call_with_context(
                &self.component,
                interface,
                Arc::from(function),
                args,
                self.context.clone(),
            )
            .await
    }
}

struct HandleLease {
    app: Arc<AppInner>,
    key: (String, &'static str),
}

impl Drop for HandleLease {
    fn drop(&mut self) {
        let mut counts = lock_or_recover(&self.app.handle_counts);
        if let Some(count) = counts.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.key);
            }
        }
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
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
    /// Exported interfaces define resources, which cannot yet be routed between components.
    ResourceExports(Vec<String>),
    /// One or more imported interfaces have no host provider.
    MissingImports(MissingImports),
    /// Loading would leave one or more component imports ambiguous.
    WouldMakeAmbiguous {
        /// Imports whose resolution would be ambiguous.
        issues: Vec<ResolutionIssue>,
    },
    /// The selected engine could not compile the component.
    Compile(EngineError),
}

impl Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnnamedComponent => formatter.write_str("component has no application name"),
            Self::DuplicateName(name) => write!(formatter, "component `{name}` is already loaded"),
            Self::ResourceExports(interfaces) => write!(
                formatter,
                "component exports unsupported resources in: {}",
                interfaces.join(", ")
            ),
            Self::MissingImports(error) => Display::fmt(error, formatter),
            Self::WouldMakeAmbiguous { issues } => write!(
                formatter,
                "load would make imports ambiguous: {}",
                issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Self::Compile(error) => write!(formatter, "component compilation failed: {error}"),
        }
    }
}

impl Error for LoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingImports(error) => Some(error),
            Self::Compile(error) => Some(error),
            Self::UnnamedComponent
            | Self::DuplicateName(_)
            | Self::ResourceExports(_)
            | Self::WouldMakeAmbiguous { .. } => None,
        }
    }
}

fn reject_resource_exports(mut interfaces: Vec<String>) -> Result<(), LoadError> {
    if interfaces.is_empty() {
        return Ok(());
    }
    interfaces.sort();
    interfaces.dedup();
    Err(LoadError::ResourceExports(interfaces))
}

/// A failure to replace one or more loaded components.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReloadError {
    /// No component is loaded under the requested name.
    UnknownComponent(String),
    /// Exported interfaces define resources, which cannot yet be routed between components.
    ResourceExports(Vec<String>),
    /// One or more imported interfaces have no unique provider.
    MissingImports(MissingImports),
    /// The replacement drops exports used by other components or live handles.
    Breaking {
        /// The component being replaced.
        component: String,
        /// Human-readable descriptions of each dependent.
        dependents: Vec<String>,
    },
    /// Replacing the component would make existing imports ambiguous.
    WouldMakeAmbiguous {
        /// Imports whose resolution would be ambiguous.
        issues: Vec<ResolutionIssue>,
    },
    /// The selected engine could not compile the replacement.
    Compile(EngineError),
}

impl Display for ReloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownComponent(name) => write!(formatter, "component `{name}` is not loaded"),
            Self::ResourceExports(interfaces) => write!(
                formatter,
                "component exports unsupported resources in: {}",
                interfaces.join(", ")
            ),
            Self::MissingImports(error) => Display::fmt(error, formatter),
            Self::Breaking {
                component,
                dependents,
            } => write!(
                formatter,
                "breaking reload of `{component}` refused; dependents: {}",
                dependents.join(", ")
            ),
            Self::WouldMakeAmbiguous { issues } => write!(
                formatter,
                "reload would make imports ambiguous: {}",
                issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Self::Compile(error) => write!(formatter, "component compilation failed: {error}"),
        }
    }
}

impl Error for ReloadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingImports(error) => Some(error),
            Self::Compile(error) => Some(error),
            Self::UnknownComponent(_)
            | Self::ResourceExports(_)
            | Self::Breaking { .. }
            | Self::WouldMakeAmbiguous { .. } => None,
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

/// A provider considered while resolving a component import.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Candidate {
    /// A provider registered by the host application.
    Host,
    /// A loaded component with the given application name.
    Component(String),
}

/// The reason a component import cannot resolve uniquely.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssueKind {
    /// No provider exports a compatible interface.
    Missing,
    /// More than one provider exports a compatible interface.
    Ambiguous {
        /// Providers that could receive the import.
        candidates: Vec<Candidate>,
    },
}

/// One component import that cannot resolve uniquely.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolutionIssue {
    /// The component containing the import.
    pub component: String,
    /// The imported interface.
    pub interface: String,
    /// The import's resolution failure.
    pub kind: IssueKind,
}

impl ResolutionIssue {
    fn missing(component: String, interface: String) -> Self {
        Self {
            component,
            interface,
            kind: IssueKind::Missing,
        }
    }

    fn ambiguous(component: String, interface: String, mut candidates: Vec<Candidate>) -> Self {
        candidates.sort();
        Self {
            component,
            interface,
            kind: IssueKind::Ambiguous { candidates },
        }
    }
}

impl Display for ResolutionIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            IssueKind::Missing => write!(
                formatter,
                "component `{}` has no provider for `{}`",
                self.component, self.interface
            ),
            IssueKind::Ambiguous { candidates } => write!(
                formatter,
                "component `{}` import `{}` is ambiguous: {}",
                self.component,
                self.interface,
                display_candidates(candidates)
            ),
        }
    }
}

fn display_candidates(candidates: &[Candidate]) -> String {
    candidates
        .iter()
        .map(|candidate| match candidate {
            Candidate::Host => String::from("`host`"),
            Candidate::Component(name) => format!("`{name}`"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Import resolution problems found by [`App::check`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckError {
    issues: Vec<ResolutionIssue>,
}

impl CheckError {
    /// Returns diagnostics naming each affected component and interface.
    #[must_use]
    pub fn issues(&self) -> &[ResolutionIssue] {
        &self.issues
    }
}

impl Display for CheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} import resolution issue(s)",
            self.issues.len()
        )
    }
}

impl Error for CheckError {}

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
    max_call_depth: Option<usize>,
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

    /// Sets the maximum number of nested component-to-component calls.
    #[must_use]
    pub fn max_call_depth(mut self, depth: usize) -> Self {
        self.max_call_depth = Some(depth);
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
            max_call_depth: self.max_call_depth.unwrap_or(64),
            components: Mutex::new(BTreeMap::new()),
            handle_counts: Mutex::new(HashMap::new()),
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

fn resolution_candidates_excluding(
    providers: &HashMap<&'static str, Arc<dyn Provider>>,
    components: &BTreeMap<String, LoadedComponent>,
    requested: &str,
    excluded: Option<&str>,
) -> Vec<Candidate> {
    providers
        .keys()
        .filter(|provided| interfaces_compatible(requested, provided))
        .map(|_| Candidate::Host)
        .chain(
            components
                .values()
                .filter(|component| excluded.is_none_or(|name| component.name.as_ref() != name))
                .filter(|component| component.exports_interface(requested))
                .map(|component| Candidate::Component(component.name.to_string())),
        )
        .collect()
}

fn load_resolution_result(
    mut missing: Vec<String>,
    mut issues: Vec<ResolutionIssue>,
) -> Result<(), LoadError> {
    missing.sort();
    missing.dedup();
    issues.sort_by(|left, right| {
        (&left.component, &left.interface).cmp(&(&right.component, &right.interface))
    });
    issues.dedup();
    if !missing.is_empty() {
        Err(LoadError::MissingImports(MissingImports::new(missing)))
    } else if !issues.is_empty() {
        Err(LoadError::WouldMakeAmbiguous { issues })
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::future::Future;
    use std::sync::{Arc, Mutex};
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

    struct RecordEvents(Arc<Mutex<Vec<Event>>>);

    impl Middleware for RecordEvents {
        async fn call(&self, call: Call, next: crate::Next) -> Result<Vals, CallError> {
            next.run(call).await
        }

        fn event(&self, event: &Event) {
            self.0.lock().unwrap().push(event.clone());
        }
    }

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
    fn handle_clones_share_one_live_interface_lease() {
        let app = App::builder().engine(ExplicitEngine).build().unwrap();
        let handle = Handle::new(app.clone(), Arc::from("journal"), "example:journal/notes");
        let clone = handle.clone();
        let key = ("journal".to_owned(), "example:journal/notes");
        assert_eq!(app.0.handle_counts.lock().unwrap().get(&key), Some(&1));

        drop(handle);
        assert_eq!(app.0.handle_counts.lock().unwrap().get(&key), Some(&1));
        drop(clone);
        assert!(!app.0.handle_counts.lock().unwrap().contains_key(&key));
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

    #[test]
    fn engine_channel_events_reach_middleware() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let app = App::builder()
            .engine(ExplicitEngine)
            .middleware(RecordEvents(events.clone()))
            .build()
            .unwrap();

        ImportDispatcher::channel_open(&app, 7, crate::ChannelDirection::HostToGuest);
        ImportDispatcher::channel_close(&app, 7, crate::ChannelDirection::HostToGuest);

        assert_eq!(
            *events.lock().unwrap(),
            [
                Event::ChannelOpen {
                    stream: 7,
                    direction: crate::ChannelDirection::HostToGuest,
                },
                Event::ChannelClose {
                    stream: 7,
                    direction: crate::ChannelDirection::HostToGuest,
                },
            ]
        );
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
    fn resolution_issue_display_names_candidates() {
        let issue = ResolutionIssue {
            component: "writer".into(),
            interface: "example:translate/translator@0.1.0".into(),
            kind: IssueKind::Ambiguous {
                candidates: vec![
                    Candidate::Component("deepl".into()),
                    Candidate::Component("google".into()),
                ],
            },
        };
        assert_eq!(
            issue.to_string(),
            "component `writer` import `example:translate/translator@0.1.0` is ambiguous: `deepl`, `google`"
        );
    }

    #[test]
    fn ambiguous_load_display_separates_resolution_issues() {
        let issue = |component: &str| ResolutionIssue {
            component: component.into(),
            interface: "example:translate/translator@0.1.0".into(),
            kind: IssueKind::Ambiguous {
                candidates: vec![
                    Candidate::Component("deepl".into()),
                    Candidate::Component("google".into()),
                ],
            },
        };
        let writer = issue("writer");
        assert_eq!(
            LoadError::WouldMakeAmbiguous {
                issues: vec![writer.clone()]
            }
            .to_string(),
            "load would make imports ambiguous: component `writer` import `example:translate/translator@0.1.0` is ambiguous: `deepl`, `google`"
        );
        assert_eq!(
            LoadError::WouldMakeAmbiguous {
                issues: vec![writer, issue("reviewer")]
            }
            .to_string(),
            "load would make imports ambiguous: component `writer` import `example:translate/translator@0.1.0` is ambiguous: `deepl`, `google`; component `reviewer` import `example:translate/translator@0.1.0` is ambiguous: `deepl`, `google`"
        );
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
