//! Shared handwritten bindings for integration tests.

#![allow(
    dead_code,
    reason = "each integration test uses a different subset of the shared bindings"
)]

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex, Weak};
use std::task::{Context, Poll, Waker};

use wasm_junction::{
    BoxFuture, Call, CallContext, CallError, Caller, CompiledComponent, Engine, EngineError,
    ImportDispatcher, InvocationContext, Provider, TypeError, TypedCall, Val, Vals, WasiConfig,
};
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

/// The interface implemented by the handwritten notes fixtures.
pub const NOTES: &str = "example:journal/notes@0.1.0";

/// The generated-handle fixture interface.
pub const HANDLE_SUMMARIES: &str = "test:handles/summaries@1.0.0";

/// The reserved interface-name fixture.
pub const RESERVED_HOST: &str = "test:keywords/host";

/// The interface used to verify invocation context propagation.
pub const CONTEXT_TARGET: &str = "example:context/target@1.0.0";

/// The host-resource bindgen fixture's exported interface.
pub const RESOURCE_BINDGEN_CLIENT: &str = "test:resource-plugin/client@1.0.0";

/// The host-resource bindgen fixture's imported interface.
pub const RESOURCE_BINDGEN_HOST: &str = "test:resources/resources@1.0.0";

/// The generated stream fixture's imported interface.
pub const STREAM_BINDGEN_HOST: &str = "test:bindgen-streams/host@1.0.0";

/// The generated stream fixture's exported interface.
pub const STREAM_BINDGEN_GUEST: &str = "test:bindgen-streams/guest@1.0.0";

/// Per-invocation data checked by the context propagation fixture.
pub struct ContextMarker(pub u32);

/// Data attached by middleware in context propagation tests.
pub struct MiddlewareMarker(pub u32);

/// Builds a real component from inline WIT and a matching dummy core module.
pub fn component_bytes(wit: &str, world_name: &str) -> Vec<u8> {
    component_bytes_from(&[("fixture.wit", wit)], world_name)
}

/// Builds a component from multiple WIT packages.
pub fn component_bytes_from(packages: &[(&str, &str)], world_name: &str) -> Vec<u8> {
    let mut resolve = Resolve::default();
    let packages = packages
        .iter()
        .map(|(name, wit)| resolve.push_str(name, wit).unwrap())
        .collect::<Vec<_>>();
    let world = resolve.select_world(&packages, Some(world_name)).unwrap();
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

/// Drives a fixture future that does not depend on an executor.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

/// An engine that compiles to an export which is never called.
pub struct FakeEngine;

impl Engine for FakeEngine {
    fn compile(
        &self,
        _bytes: Arc<[u8]>,
        _wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        Box::pin(async { Ok(Arc::new(UnusedComponent) as Arc<dyn CompiledComponent>) })
    }
}

/// An engine that succeeds a configured number of times before failing.
pub struct FailingEngine {
    successes: AtomicUsize,
    fallback: FakeEngine,
}

impl FailingEngine {
    /// Creates an engine that fails after `successes` compilations.
    pub fn after(successes: usize) -> Self {
        Self {
            successes: AtomicUsize::new(successes),
            fallback: FakeEngine,
        }
    }
}

impl Engine for FailingEngine {
    fn compile(
        &self,
        bytes: Arc<[u8]>,
        wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        if self
            .successes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                count.checked_sub(1)
            })
            .is_ok()
        {
            self.fallback.compile(bytes, wasi)
        } else {
            Box::pin(async { Err(EngineError::new("invalid adapter")) })
        }
    }
}

/// Shared observations and synchronization for [`GenerationEngine`].
#[derive(Default)]
pub struct GenerationState {
    compilations: AtomicUsize,
    generations: Mutex<Vec<Weak<GenerationComponent>>>,
    gate: Gate,
}

impl GenerationState {
    /// Waits until the first generation has entered its call.
    pub fn wait_until_called(&self) {
        self.gate.wait_until_entered();
    }

    /// Lets the first generation's call finish.
    pub fn release(&self) {
        self.gate.release();
    }

    /// Returns a weak reference to the compiled generation at `index`.
    pub fn generation(&self, index: usize) -> Weak<GenerationComponent> {
        self.generations.lock().unwrap()[index].clone()
    }
}

/// An engine whose successive generations return `old` and `new`.
pub struct GenerationEngine(pub Arc<GenerationState>);

impl Engine for GenerationEngine {
    fn compile(
        &self,
        _bytes: Arc<[u8]>,
        _wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        let version = self.0.compilations.fetch_add(1, Ordering::SeqCst);
        let component = Arc::new(GenerationComponent {
            value: if version == 0 { "old" } else { "new" },
            gate: (version == 0).then(|| self.0.gate.clone()),
        });
        self.0
            .generations
            .lock()
            .unwrap()
            .push(Arc::downgrade(&component));
        Box::pin(async move { Ok(component as Arc<dyn CompiledComponent>) })
    }
}

/// A generation engine that routes writers through successive translator generations.
pub struct RoutingGenerationEngine(pub Arc<GenerationState>);

impl Engine for RoutingGenerationEngine {
    fn compile(
        &self,
        bytes: Arc<[u8]>,
        _wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        if bytes
            .windows(b"example:writer/article".len())
            .any(|window| window == b"example:writer/article")
        {
            return Box::pin(async { Ok(Arc::new(WriterComponent) as Arc<dyn CompiledComponent>) });
        }
        let version = self.0.compilations.fetch_add(1, Ordering::SeqCst);
        let component = Arc::new(GenerationComponent {
            value: if version == 0 { "old" } else { "new" },
            gate: (version == 0).then(|| self.0.gate.clone()),
        });
        self.0
            .generations
            .lock()
            .unwrap()
            .push(Arc::downgrade(&component));
        Box::pin(async move { Ok(component as Arc<dyn CompiledComponent>) })
    }
}

/// Shared gate for an engine whose replacement compilation suspends.
#[derive(Default)]
pub struct CompileState {
    compilations: AtomicUsize,
    gate: Gate,
}

impl CompileState {
    /// Waits until replacement compilation has started.
    pub fn wait_until_compiling(&self) {
        self.gate.wait_until_entered();
    }

    /// Lets replacement compilation finish.
    pub fn release(&self) {
        self.gate.release();
    }
}

/// An engine that gates its second compilation while the old component stays callable.
pub struct GatedCompileEngine(pub Arc<CompileState>);

impl Engine for GatedCompileEngine {
    fn compile(
        &self,
        _bytes: Arc<[u8]>,
        _wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        let replacement = self.0.compilations.fetch_add(1, Ordering::SeqCst) > 0;
        let gate = self.0.gate.clone();
        Box::pin(async move {
            if replacement {
                gate.enter();
            }
            Ok(Arc::new(GenerationComponent {
                value: if replacement { "new" } else { "old" },
                gate: None,
            }) as Arc<dyn CompiledComponent>)
        })
    }
}

/// An engine that releases two replacement compilations at the same time.
pub struct ConcurrentCompileEngine {
    compilations: AtomicUsize,
    replacements: Barrier,
}

impl Default for ConcurrentCompileEngine {
    fn default() -> Self {
        Self {
            compilations: AtomicUsize::new(0),
            replacements: Barrier::new(2),
        }
    }
}

impl Engine for ConcurrentCompileEngine {
    fn compile(
        &self,
        _bytes: Arc<[u8]>,
        _wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        let replacement = self.compilations.fetch_add(1, Ordering::SeqCst) > 0;
        Box::pin(async move {
            if replacement {
                self.replacements.wait();
            }
            Ok(Arc::new(UnusedComponent) as Arc<dyn CompiledComponent>)
        })
    }
}

struct WriterComponent;

impl CompiledComponent for WriterComponent {
    fn call(
        &self,
        imports: Arc<dyn ImportDispatcher>,
        context: InvocationContext,
        component: Arc<str>,
        _interface: Arc<str>,
        _function: Arc<str>,
        args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(async move {
            imports
                .call(
                    context,
                    component,
                    Arc::from("example:translate/translator@0.1.0"),
                    Arc::from("translate"),
                    args,
                )
                .await
        })
    }
}

#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<(bool, bool)>, Condvar)>);

impl Gate {
    fn enter(&self) {
        let (lock, changed) = &*self.0;
        let mut state = lock.lock().unwrap();
        state.0 = true;
        changed.notify_all();
        while !state.1 {
            state = changed.wait(state).unwrap();
        }
    }

    fn wait_until_entered(&self) {
        let (lock, changed) = &*self.0;
        let mut state = lock.lock().unwrap();
        while !state.0 {
            state = changed.wait(state).unwrap();
        }
    }

    fn release(&self) {
        let (lock, changed) = &*self.0;
        lock.lock().unwrap().1 = true;
        changed.notify_all();
    }
}

/// A compiled generation observable through a weak reference.
pub struct GenerationComponent {
    value: &'static str,
    gate: Option<Gate>,
}

impl CompiledComponent for GenerationComponent {
    fn call(
        &self,
        _imports: Arc<dyn ImportDispatcher>,
        _context: InvocationContext,
        _component: Arc<str>,
        _interface: Arc<str>,
        _function: Arc<str>,
        _args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(async move {
            if let Some(gate) = &self.gate {
                gate.enter();
            }
            Ok(vec![Val::from(self.value)])
        })
    }
}

struct UnusedComponent;

impl CompiledComponent for UnusedComponent {
    fn call(
        &self,
        imports: Arc<dyn ImportDispatcher>,
        context: InvocationContext,
        component: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(async move {
            match interface.as_ref() {
                "example:cycle/first-api@1.0.0" => {
                    imports
                        .call(
                            context,
                            component,
                            Arc::from("example:cycle/second-api@1.0.0"),
                            function,
                            args,
                        )
                        .await
                }
                "example:cycle/second-api@1.0.0" => {
                    imports
                        .call(
                            context,
                            component,
                            Arc::from("example:cycle/first-api@1.0.0"),
                            function,
                            args,
                        )
                        .await
                }
                "example:writer/article@1.0.0" => {
                    imports
                        .call(
                            context,
                            component,
                            Arc::from("example:translate/translator@0.1.0"),
                            Arc::from("translate"),
                            args,
                        )
                        .await
                }
                "example:translate/translator@0.1.7" => {
                    let [wasm_junction::Val::String(text)] = args.as_slice() else {
                        return Err(CallError::trap("translator expected one string"));
                    };
                    Ok(vec![format!("translated: {text}").into()])
                }
                CONTEXT_TARGET => {
                    let marker = context
                        .extensions()
                        .get::<ContextMarker>()
                        .ok_or_else(|| CallError::trap("invocation context marker is missing"))?;
                    let added = context
                        .extensions()
                        .get::<MiddlewareMarker>()
                        .map_or(0, |marker| marker.0);
                    Ok(vec![Val::U32(marker.0), Val::U32(added)])
                }
                RESOURCE_BINDGEN_CLIENT => {
                    let import = resource_import(&function)?;
                    imports
                        .call(
                            context,
                            component,
                            Arc::from(RESOURCE_BINDGEN_HOST),
                            Arc::from(import),
                            args,
                        )
                        .await
                }
                STREAM_BINDGEN_GUEST => {
                    stream_import(imports, context, component, function, args).await
                }
                RESERVED_HOST => {
                    imports
                        .call(context, component, interface, function, args)
                        .await
                }
                HANDLE_SUMMARIES if function.as_ref() == "context" => Ok(vec![Val::U32(
                    context
                        .extensions()
                        .get::<ContextMarker>()
                        .map_or(0, |marker| marker.0),
                )]),
                HANDLE_SUMMARIES => handle_call(&function, args),
                "example:journal/summaries@0.1.0" => {
                    let import = match function.as_ref() {
                        "summarize" => "read",
                        "search" => "search",
                        "clear" => "clear",
                        "unknown" => "unknown",
                        _ => return Err(CallError::trap("engine received the wrong function")),
                    };
                    imports
                        .call(
                            context,
                            component,
                            Arc::from(NOTES),
                            Arc::from(import),
                            args,
                        )
                        .await
                }
                _ => Err(CallError::trap("engine received an unresolved export")),
            }
        })
    }
}

async fn stream_import(
    imports: Arc<dyn ImportDispatcher>,
    context: InvocationContext,
    component: Arc<str>,
    function: Arc<str>,
    args: Vals,
) -> Result<Vals, CallError> {
    imports
        .call(
            context,
            component,
            Arc::from(STREAM_BINDGEN_HOST),
            function,
            args,
        )
        .await
}

fn resource_import(function: &str) -> Result<&str, CallError> {
    match function {
        "open" => Ok("[constructor]session"),
        "profile" => Ok("[method]session.profile"),
        "new" => Ok("[method]session.new"),
        "lookup" => Ok("[static]session.lookup"),
        "consume" | "maybe" | "choose" | "make-host" => Ok(function),
        _ => Err(CallError::trap("unknown resource fixture function")),
    }
}

fn handle_call(function: &str, args: Vals) -> Result<Vals, CallError> {
    match function {
        "summarize" => {
            let expected = vec![
                Val::String("today".into()),
                Val::Record(vec![("title".into(), Val::String("project".into()))]),
                Val::Variant {
                    case: "one".into(),
                    value: Some(Box::new(Val::String("open".into()))),
                },
                Val::List(vec![Val::U32(2), Val::U32(4)]),
                Val::U32(10),
            ];
            if args != expected {
                return Err(CallError::trap("handle arguments were encoded incorrectly"));
            }
            Ok(vec![Val::String("summary".into())])
        }
        "accepted" => Ok(vec![Val::Result(Ok(Some(Box::new(Val::String(
            "saved".into(),
        )))))]),
        "rejected" => Ok(vec![Val::Result(Err(Some(Box::new(Val::String(
            "denied".into(),
        )))))]),
        "clone" | "f" | "from-app" | "g" | "with" | "within" => Ok(args),
        "inspect" => {
            let expected = vec![
                Val::Result(Ok(Some(Box::new(Val::String("done".into()))))),
                Val::Tuple(vec![Val::String("pair".into()), Val::U32(7)]),
            ];
            if args != expected {
                return Err(CallError::trap(
                    "borrowed arguments were encoded incorrectly",
                ));
            }
            Ok(Vec::new())
        }
        _ => Err(CallError::trap("engine received the wrong function")),
    }
}

/// A provider used only to satisfy a component import.
pub struct UnusedProvider;

impl Provider for UnusedProvider {
    fn call<'a>(
        &'a self,
        _cx: &'a CallContext,
        _call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async { Err(CallError::trap("unused provider")) })
    }
}

/// Typed arguments for the notes `read` function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// The note name to read.
    pub name: String,
}

impl TypedCall for Read {
    type Output = String;

    const INTERFACE: &'static str = NOTES;
    const FUNCTION: &'static str = "read";

    fn from_vals(values: &[Val]) -> Result<Self, TypeError> {
        let [Val::String(name)] = values else {
            return Err(TypeError::new("notes.read expects one string"));
        };
        Ok(Self { name: name.clone() })
    }

    fn into_vals(self) -> Vals {
        vec![self.name.into()]
    }

    fn output(value: Self::Output) -> Vals {
        vec![value.into()]
    }

    fn decode_output(values: &[Val]) -> Result<Self::Output, TypeError> {
        let [Val::String(value)] = values else {
            return Err(TypeError::new("notes.read returns one string"));
        };
        Ok(value.clone())
    }
}

/// Typed arguments for the notes `delete` function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delete {
    /// The note name to delete.
    pub name: String,
}

impl TypedCall for Delete {
    type Output = ();

    const INTERFACE: &'static str = NOTES;
    const FUNCTION: &'static str = "delete";

    fn from_vals(values: &[Val]) -> Result<Self, TypeError> {
        let [Val::String(name)] = values else {
            return Err(TypeError::new("notes.delete expects one string"));
        };
        Ok(Self { name: name.clone() })
    }

    fn into_vals(self) -> Vals {
        vec![self.name.into()]
    }

    fn output(_value: Self::Output) -> Vals {
        Vec::new()
    }

    fn decode_output(values: &[Val]) -> Result<Self::Output, TypeError> {
        if values.is_empty() {
            Ok(())
        } else {
            Err(TypeError::new("notes.delete returns no values"))
        }
    }
}

/// Builds a notes `read` invocation from the summarizer component.
pub fn read_call(name: &str) -> Call {
    Call::new(
        Caller::Component(Arc::from("summarizer")),
        "notebook",
        NOTES,
        "read",
        vec![name.into()],
    )
}
