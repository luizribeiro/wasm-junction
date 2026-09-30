//! Application loading tests.

mod support;

use std::sync::{Arc, Mutex};

use support::{
    FakeEngine, NOTES, Read, UnusedProvider, block_on, component_bytes, component_bytes_from,
};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, Caller, Component, GetError, InterfaceHandle, LoadError,
    Middleware, Next, Provided, Provider, Trap, TypedCall, Vals,
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

fn component(name: &str) -> Component {
    Component::from_bytes(component_bytes(PLUGIN_WIT, "plugin"))
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
    async fn summarize(&self, note: &str) -> Result<Vals, Trap> {
        self.app
            .call(&self.component, SUMMARIES, "summarize", vec![note.into()])
            .await
    }
}

struct Trace(Arc<Mutex<Vec<Call>>>);

impl Middleware for Trace {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, Trap> {
        self.0.lock().unwrap().push(call.clone());
        next.run(call).await
    }
}

struct NotesProvider(Arc<Mutex<Vec<Caller>>>);

impl Provider for NotesProvider {
    fn call<'a>(&'a self, cx: &'a CallContext, call: Call) -> BoxFuture<'a, Result<Vals, Trap>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(cx.caller().clone());
            let read = call
                .view::<Read>()?
                .ok_or_else(|| Trap::new("unknown notes function"))?;
            Ok(Read::output(format!("contents of {}", read.name)))
        })
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
