//! Application loading tests.

mod support;

use std::sync::{Arc, Mutex};

use support::{
    FakeEngine, NOTES, Read, UnusedProvider, block_on, component_bytes, component_bytes_from,
};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CallErrorKind, Caller, CompiledComponent,
    Component, Engine, EngineError, GetError, InterfaceHandle, LoadError, Middleware, Next,
    Provided, Provider, TypedCall, Vals, WasiConfig,
};

const CLOCK: &str = "example:journal/clock@0.1.0";
const SUMMARIES: &str = "example:journal/summaries@0.1.9";
const PLUGIN_WIT: &str = r"
    package example:journal@0.1.0;
    interface notes { read: func(name: string) -> string; }
    interface clock { now: func() -> u64; }
    interface summaries { summarize: func(note: string) -> string; }
    world plugin { import notes; import clock; export summaries; }
";
const TYPES_WIT: &str = r"
    package example:shared@0.1.0;
    interface types { record note { text: string } }
";
const TYPES_PLUGIN_WIT: &str = r"
    package example:typed@0.1.0;
    interface summaries {
        use example:shared/types@0.1.0.{note};
        summarize: func(note: note) -> string;
    }
    world plugin { export summaries; }
";
const TRANSLATOR_WIT: &str = r"
    package example:translate@0.1.7;
    interface translator { translate: func(text: string) -> string; }
    world service { export translator; }
";
const TRANSLATOR_API_WIT: &str = r"
    package example:translate@0.1.0;
    interface translator { translate: func(text: string) -> string; }
";
const WRITER_WIT: &str = r"
    package example:writer@1.0.0;
    interface article { write: func(text: string) -> string; }
    world writer {
        import example:translate/translator@0.1.0;
        export article;
    }
";

fn component(name: &str) -> Component {
    Component::from_bytes(component_bytes(PLUGIN_WIT, "plugin"))
        .unwrap()
        .named(name)
}

fn wit_component(wit: &str, world: &str, name: &str) -> Component {
    Component::from_bytes(component_bytes(wit, world))
        .unwrap()
        .named(name)
}

fn writer_component(name: &str) -> Component {
    Component::from_bytes(component_bytes_from(
        &[
            ("translator.wit", TRANSLATOR_API_WIT),
            ("writer.wit", WRITER_WIT),
        ],
        "example:writer/writer@1.0.0",
    ))
    .unwrap()
    .named(name)
}

#[derive(Clone)]
struct Summaries {
    app: App,
    component: Arc<str>,
}

impl InterfaceHandle for Summaries {
    const INTERFACE: &'static str = SUMMARIES;

    fn from_app(app: App, component: Arc<str>) -> Self {
        Self { app, component }
    }
}

impl Summaries {
    async fn summarize(&self, note: &str) -> Result<Vals, CallError> {
        self.app
            .call(&self.component, SUMMARIES, "summarize", vec![note.into()])
            .await
    }
}

struct Trace(Arc<Mutex<Vec<Call>>>);

impl Middleware for Trace {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        self.0.lock().unwrap().push(call.clone());
        next.run(call).await
    }
}

struct NotesProvider(Arc<Mutex<Vec<Caller>>>);

impl Provider for NotesProvider {
    fn call<'a>(
        &'a self,
        cx: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(cx.caller().clone());
            let read = call
                .view::<Read>()?
                .ok_or_else(|| CallError::trap("unknown notes function"))?;
            Ok(Read::output(format!("contents of {}", read.name)))
        })
    }
}

struct FailingEngine;

impl Engine for FailingEngine {
    fn compile(
        &self,
        _bytes: Arc<[u8]>,
        _wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
        Box::pin(async { Err(EngineError::new("invalid adapter")) })
    }
}

struct Notes;

impl InterfaceHandle for Notes {
    const INTERFACE: &'static str = NOTES;

    fn from_app(_app: App, _component: Arc<str>) -> Self {
        Self
    }
}

#[test]
fn load_compiles_and_refuses_missing_imports_or_duplicate_names() {
    let missing = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new(NOTES, UnusedProvider))
        .build()
        .unwrap();
    let error = block_on(missing.load(component("summarizer"))).unwrap_err();
    let LoadError::MissingImports(missing) = error else {
        panic!("expected missing imports");
    };
    assert_eq!(missing.interfaces(), [CLOCK]);

    let app = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new(NOTES, UnusedProvider))
        .provide(Provided::new(CLOCK, UnusedProvider))
        .build()
        .unwrap();
    block_on(app.load(component("summarizer"))).unwrap();
    assert!(matches!(
        block_on(app.load(component("summarizer"))),
        Err(LoadError::DuplicateName(name)) if name == "summarizer"
    ));
}

#[test]
fn loaded_component_exports_satisfy_compatible_imports() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "translator"))).unwrap();
    block_on(app.load(writer_component("writer"))).unwrap();
}

#[test]
fn component_imports_route_through_middleware() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(FakeEngine)
        .middleware(Trace(trace.clone()))
        .build()
        .unwrap();
    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "translator"))).unwrap();
    block_on(app.load(writer_component("writer"))).unwrap();

    let result = block_on(app.call(
        "writer",
        "example:writer/article@1.0.0",
        "write",
        vec!["hello".into()],
    ))
    .unwrap();
    assert_eq!(result, [wasm_junction::Val::from("translated: hello")]);
    let calls = trace.lock().unwrap();
    assert_eq!(calls[1].caller, Caller::Component(Arc::from("writer")));
    assert_eq!(calls[1].callee.as_ref(), "translator");
}

#[test]
fn explicit_link_selects_one_component_provider() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(FakeEngine)
        .middleware(Trace(trace.clone()))
        .build()
        .unwrap();
    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "deepl"))).unwrap();
    block_on(app.load(writer_component("writer"))).unwrap();
    app.link("writer", "example:translate/translator@0.1.0", "deepl")
        .unwrap();
    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "google"))).unwrap();
    block_on(app.call(
        "writer",
        "example:writer/article@1.0.0",
        "write",
        vec!["hello".into()],
    ))
    .unwrap();
    assert_eq!(trace.lock().unwrap()[1].callee.as_ref(), "deepl");
}

#[test]
fn ambiguous_call_errors_name_component_candidates() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "deepl"))).unwrap();
    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "google"))).unwrap();
    block_on(app.load(writer_component("writer"))).unwrap();

    let error = block_on(app.call(
        "writer",
        "example:writer/article@1.0.0",
        "write",
        vec!["hello".into()],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(
        error.to_string(),
        "more than one provider for `example:translate/translator@0.1.0`: `deepl`, `google`"
    );
}

#[test]
fn typed_handle_queries_report_names_and_export_mismatches() {
    let app = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new(NOTES, UnusedProvider))
        .provide(Provided::new(CLOCK, UnusedProvider))
        .build()
        .unwrap();
    block_on(app.load(component("work"))).unwrap();
    block_on(app.load(component("home"))).unwrap();

    assert!(app.has::<Summaries>("work"));
    assert!(!app.has::<Notes>("work"));
    assert_eq!(
        app.all::<Summaries>()
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["home", "work"]
    );
    assert!(matches!(
        app.get::<Summaries>("missing"),
        Err(GetError::UnknownComponent(name)) if name == "missing"
    ));
    assert!(matches!(
        app.get::<Notes>("work"),
        Err(GetError::MissingExport { component, interface })
            if component == "work" && interface == NOTES
    ));
}

#[test]
fn export_and_guest_import_calls_share_the_middleware_dispatcher() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let callers = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new(NOTES, NotesProvider(callers.clone())))
        .provide(Provided::new(CLOCK, UnusedProvider))
        .middleware(Trace(trace.clone()))
        .build()
        .unwrap();
    block_on(app.load(component("summarizer"))).unwrap();

    let summaries = app.get::<Summaries>("summarizer").unwrap();
    assert_eq!(
        block_on(summaries.summarize("daily")).unwrap(),
        [wasm_junction::Val::from("contents of daily")]
    );
    let calls = trace.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].caller, Caller::Host);
    assert_eq!(
        calls[0].interface.as_ref(),
        "example:journal/summaries@0.1.0"
    );
    assert_eq!(calls[1].caller, Caller::Component(Arc::from("summarizer")));
    assert_eq!(
        *callers.lock().unwrap(),
        [Caller::Component(Arc::from("summarizer"))]
    );
}

#[test]
fn raw_calls_accept_function_names_built_at_runtime() {
    let app = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new(NOTES, NotesProvider(Arc::default())))
        .provide(Provided::new(CLOCK, UnusedProvider))
        .build()
        .unwrap();
    block_on(app.load(component("summarizer"))).unwrap();

    let function = String::from("summarize");
    assert_eq!(
        block_on(app.call("summarizer", SUMMARIES, function, vec!["daily".into()])).unwrap(),
        [wasm_junction::Val::from("contents of daily")]
    );
}

#[test]
fn calls_report_unavailable_components() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    let error = block_on(app.call("missing", SUMMARIES, "summarize", Vec::new())).unwrap_err();

    assert_eq!(error.kind(), CallErrorKind::Unavailable);
    assert_eq!(error.to_string(), "component `missing` is not loaded");
}

#[test]
fn type_only_imports_do_not_require_providers() {
    let bytes = component_bytes_from(
        &[("types.wit", TYPES_WIT), ("plugin.wit", TYPES_PLUGIN_WIT)],
        "example:typed/plugin@0.1.0",
    );
    let component = Component::from_bytes(bytes).unwrap().named("typed");
    assert!(component.imports().is_empty());
    assert_eq!(component.type_imports(), ["example:shared/types@0.1.0"]);

    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(app.load(component)).unwrap();
}

#[test]
fn engine_compilation_errors_are_distinct_from_call_traps() {
    let app = App::builder()
        .engine(FailingEngine)
        .provide(Provided::new(NOTES, UnusedProvider))
        .provide(Provided::new(CLOCK, UnusedProvider))
        .build()
        .unwrap();

    assert!(matches!(
        block_on(app.load(component("broken"))),
        Err(LoadError::Compile(error)) if error.to_string() == "invalid adapter"
    ));
}
