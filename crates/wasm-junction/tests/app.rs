//! Application loading tests.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use support::{
    CONTEXT_TARGET, ContextMarker, FailingEngine, FakeEngine, MiddlewareMarker, NOTES, Read,
    UnusedProvider, block_on, component_bytes, component_bytes_from,
};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CallErrorKind, Caller, Candidate, Component,
    GetError, ImportDispatcher, InterfaceHandle, InvocationContext, IssueKind, LoadError,
    Middleware, Next, OutputStream, Provided, Provider, Resource, TypedCall, Val, Vals,
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
const CYCLE_WIT: &str = r"
    package example:cycle@1.0.0;
    interface first-api { ping: func(); }
    world first-component {
        import second-api;
        export first-api;
    }
    interface second-api { ping: func(); }
    world second-component {
        import first-api;
        export second-api;
    }
";
const CONTEXT_WIT: &str = r"
    package example:context@1.0.0;
    interface target { read: func() -> u32; }
    world target-component { export target; }
";
const RESOURCE_EXPORT_WIT: &str = r"
    package example:resources@1.0.0;
    interface sessions { resource session; open: func() -> session; }
    interface files { resource file; create: func() -> file; }
    world plugin { export sessions; export files; }
";
const RESOURCE_ALIAS_WIT: &str = r"
    package example:hosted@1.0.0;
    interface host { resource session; }
    interface client {
        use host.{session};
        round-trip: func(value: session) -> session;
    }
    world plugin { import host; export client; }
";
const RESOURCE_COLLISION_WIT: &str = r"
    package example:hosted@1.0.0;
    interface host { resource session; }
    interface client { resource session; round-trip: func(value: session) -> session; }
    world plugin { import host; export client; }
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

fn cyclic_component(world: &str, name: &str) -> Component {
    Component::from_bytes(component_bytes(CYCLE_WIT, world))
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

struct AttachInvocationData;

impl Middleware for AttachInvocationData {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        assert_eq!(
            call.extensions()
                .get::<ContextMarker>()
                .map(|marker| marker.0),
            Some(42)
        );
        if call.function.as_ref() == "attach" {
            call.extensions_mut().insert(MiddlewareMarker(7));
        }
        next.run(call).await
    }
}

struct ContextProvider;

impl Provider for ContextProvider {
    fn call<'a>(
        &'a self,
        cx: &'a CallContext,
        _call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            let marker = cx.extensions().get::<MiddlewareMarker>().unwrap();
            Ok(vec![Val::U32(marker.0)])
        })
    }
}

#[derive(Clone)]
struct NotesSettings(&'static str);

struct SettingsGate {
    first: AtomicBool,
    barrier: Barrier,
}

struct SettingsProvider {
    seen: Arc<Mutex<Vec<Option<&'static str>>>>,
    gate: Option<Arc<SettingsGate>>,
}

impl Provider for SettingsProvider {
    fn call<'a>(
        &'a self,
        cx: &'a CallContext,
        _call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            if let Some(gate) = &self.gate
                && gate.first.swap(false, Ordering::SeqCst)
            {
                gate.barrier.wait();
                gate.barrier.wait();
            }
            self.seen
                .lock()
                .unwrap()
                .push(cx.settings::<NotesSettings>().map(|value| value.0));
            Ok(vec![Val::from("note")])
        })
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
fn load_refuses_interfaces_that_export_resources() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    let component = wit_component(RESOURCE_EXPORT_WIT, "plugin", "resources");
    let error = block_on(app.load(component)).unwrap_err();
    assert_eq!(
        error,
        LoadError::ResourceExports(vec![
            "example:resources/files@1.0.0".to_owned(),
            "example:resources/sessions@1.0.0".to_owned(),
        ])
    );
}

#[test]
fn load_accepts_exports_that_only_reuse_a_host_resource() {
    let app = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new("example:hosted/host@1.0.0", UnusedProvider))
        .build()
        .unwrap();
    let component = wit_component(RESOURCE_ALIAS_WIT, "plugin", "resource-client");
    block_on(app.load(component)).unwrap();
}

#[test]
fn load_refuses_a_local_resource_with_an_imported_resource_name() {
    let app = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new("example:hosted/host@1.0.0", UnusedProvider))
        .build()
        .unwrap();
    let component = wit_component(RESOURCE_COLLISION_WIT, "plugin", "resource-client");
    assert_eq!(
        block_on(app.load(component)).unwrap_err(),
        LoadError::ResourceExports(vec!["example:hosted/client@1.0.0".to_owned()])
    );
}

#[test]
fn loaded_component_exports_satisfy_compatible_imports() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "translator"))).unwrap();
    block_on(app.load(writer_component("writer"))).unwrap();

    let error = block_on(app.call(
        "writer",
        "example:writer/article@1.0.0",
        "write",
        vec![Val::Resource(Resource::owned(
            "example:host/files@1.0.0",
            "file",
            0,
        ))],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(
        error.to_string(),
        "only plain values can cross between components"
    );

    let error = block_on(app.call(
        "writer",
        "example:writer/article@1.0.0",
        "write",
        vec![OutputStream::from_bytes(b"stream").into()],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(
        error.to_string(),
        "only plain values can cross between components"
    );
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
fn middleware_data_crosses_a_component_hop_without_leaking() {
    let app = App::builder()
        .engine(FakeEngine)
        .middleware(AttachInvocationData)
        .build()
        .unwrap();
    for name in ["caller", "callee"] {
        block_on(app.load(wit_component(CONTEXT_WIT, "target-component", name))).unwrap();
    }

    let context = InvocationContext::with(ContextMarker(42));
    let attached = block_on(ImportDispatcher::call(
        &app,
        context.clone(),
        Arc::from("caller"),
        Arc::from(CONTEXT_TARGET),
        Arc::from("attach"),
        Vec::new(),
    ))
    .unwrap();
    assert_eq!(attached, [Val::U32(42), Val::U32(7)]);

    let sibling = block_on(ImportDispatcher::call(
        &app,
        context.clone(),
        Arc::from("caller"),
        Arc::from(CONTEXT_TARGET),
        Arc::from("read"),
        Vec::new(),
    ))
    .unwrap();
    assert_eq!(sibling, [Val::U32(42), Val::U32(0)]);
    assert!(context.extensions().get::<MiddlewareMarker>().is_none());
}

#[test]
fn middleware_data_reaches_provider_context() {
    let app = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new(CONTEXT_TARGET, ContextProvider))
        .middleware(AttachInvocationData)
        .build()
        .unwrap();

    let values = block_on(ImportDispatcher::call(
        &app,
        InvocationContext::with(ContextMarker(42)),
        Arc::from("caller"),
        Arc::from(CONTEXT_TARGET),
        Arc::from("attach"),
        Vec::new(),
    ))
    .unwrap();
    assert_eq!(values, [Val::U32(7)]);
}

#[test]
fn component_settings_are_type_indexed_per_name_and_replace_live() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(SettingsGate {
        first: AtomicBool::new(true),
        barrier: Barrier::new(2),
    });
    let app = App::builder()
        .engine(FakeEngine)
        .provide(Provided::new(
            NOTES,
            SettingsProvider {
                seen: seen.clone(),
                gate: Some(gate.clone()),
            },
        ))
        .provide(Provided::new(CLOCK, UnusedProvider))
        .build()
        .unwrap();
    app.configure("alpha", NotesSettings("research")).unwrap();
    for name in ["alpha", "beta"] {
        block_on(app.load(component(name))).unwrap();
    }

    let caller = app.clone();
    let in_flight = std::thread::spawn(move || {
        block_on(caller.get::<Summaries>("alpha").unwrap().summarize("daily")).unwrap()
    });
    gate.barrier.wait();
    app.configure("alpha", NotesSettings("drafts")).unwrap();
    gate.barrier.wait();
    in_flight.join().unwrap();

    block_on(app.get::<Summaries>("alpha").unwrap().summarize("daily")).unwrap();
    block_on(app.get::<Summaries>("beta").unwrap().summarize("daily")).unwrap();
    app.configure("beta", NotesSettings("archive")).unwrap();
    block_on(app.get::<Summaries>("beta").unwrap().summarize("daily")).unwrap();

    assert_eq!(
        *seen.lock().unwrap(),
        [Some("research"), Some("drafts"), None, Some("archive")]
    );
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
    app.check().unwrap();
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
fn load_refuses_to_make_an_existing_import_ambiguous() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "deepl"))).unwrap();
    block_on(app.load(writer_component("writer"))).unwrap();

    let error = block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "google"))).unwrap_err();
    let LoadError::WouldMakeAmbiguous { issues } = error else {
        panic!("expected ambiguous load refusal");
    };
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].component, "writer");
    assert_eq!(issues[0].interface, "example:translate/translator@0.1.0");
    assert_eq!(
        issues[0].to_string(),
        "component `writer` import `example:translate/translator@0.1.0` is ambiguous: `deepl`, `google`"
    );
}

#[test]
fn cyclic_components_load_in_any_order_and_stop_at_the_depth_limit() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(FakeEngine)
        .middleware(Trace(trace.clone()))
        .max_call_depth(3)
        .build()
        .unwrap();
    let first = cyclic_component("first-component", "first");
    let second = cyclic_component("second-component", "second");
    block_on(app.load_all([second, first])).unwrap();
    app.check().unwrap();
    let error = block_on(app.call("first", "example:cycle/first-api@1.0.0", "ping", Vec::new()))
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "maximum call depth of 3 exceeded");
    let calls = trace.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call.caller, Caller::Component(_)))
            .count(),
        3
    );
}

#[test]
fn load_all_inserts_nothing_when_validation_fails() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    let translator = wit_component(TRANSLATOR_WIT, "service", "translator");
    assert!(block_on(app.load_all([translator, component("writer")])).is_err());
    let error = block_on(app.call(
        "translator",
        "example:translate/translator@0.1.7",
        "translate",
        vec!["hello".into()],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Unavailable);
}

#[test]
fn load_all_refuses_batch_providers_that_ambiguate_an_existing_import() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "deepl"))).unwrap();
    block_on(app.load(writer_component("writer"))).unwrap();

    let google = wit_component(TRANSLATOR_WIT, "service", "google");
    let local = wit_component(TRANSLATOR_WIT, "service", "local");
    let error = block_on(app.load_all([google, local])).unwrap_err();
    let LoadError::WouldMakeAmbiguous { issues } = error else {
        panic!("expected ambiguous load refusal");
    };
    let IssueKind::Ambiguous { candidates } = &issues[0].kind else {
        panic!("expected candidate list");
    };
    assert_eq!(issues[0].component, "writer");
    assert_eq!(
        candidates,
        &[
            Candidate::Component("deepl".into()),
            Candidate::Component("google".into()),
            Candidate::Component("local".into()),
        ]
    );
}

#[test]
fn load_all_refuses_batch_providers_that_ambiguate_a_batch_import() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    let deepl = wit_component(TRANSLATOR_WIT, "service", "deepl");
    let google = wit_component(TRANSLATOR_WIT, "service", "google");

    let error = block_on(app.load_all([deepl, google, writer_component("writer")])).unwrap_err();
    let LoadError::WouldMakeAmbiguous { issues } = error else {
        panic!("expected ambiguous load refusal");
    };
    let IssueKind::Ambiguous { candidates } = &issues[0].kind else {
        panic!("expected candidate list");
    };
    assert_eq!(issues[0].component, "writer");
    assert_eq!(candidates.len(), 2);
}

#[test]
fn load_all_refuses_repeated_and_already_loaded_names() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    let first = wit_component(TRANSLATOR_WIT, "service", "translator");
    let repeated = wit_component(TRANSLATOR_WIT, "service", "translator");
    assert!(matches!(
        block_on(app.load_all([first, repeated])),
        Err(LoadError::DuplicateName(name)) if name == "translator"
    ));

    block_on(app.load(wit_component(TRANSLATOR_WIT, "service", "loaded"))).unwrap();
    let duplicate = wit_component(TRANSLATOR_WIT, "service", "loaded");
    assert!(matches!(
        block_on(app.load_all([duplicate])),
        Err(LoadError::DuplicateName(name)) if name == "loaded"
    ));
}

#[test]
fn load_all_inserts_nothing_when_a_later_compile_fails() {
    let app = App::builder()
        .engine(FailingEngine::after(1))
        .build()
        .unwrap();
    let first = wit_component(TRANSLATOR_WIT, "service", "first");
    let second = wit_component(TRANSLATOR_WIT, "service", "second");

    assert!(matches!(
        block_on(app.load_all([first, second])),
        Err(LoadError::Compile(error)) if error.to_string() == "invalid adapter"
    ));
    for name in ["first", "second"] {
        let error = block_on(app.call(
            name,
            "example:translate/translator@0.1.7",
            "translate",
            vec!["hello".into()],
        ))
        .unwrap_err();
        assert_eq!(error.kind(), CallErrorKind::Unavailable);
    }
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
        .engine(FailingEngine::after(0))
        .provide(Provided::new(NOTES, UnusedProvider))
        .provide(Provided::new(CLOCK, UnusedProvider))
        .build()
        .unwrap();

    assert!(matches!(
        block_on(app.load(component("broken"))),
        Err(LoadError::Compile(error)) if error.to_string() == "invalid adapter"
    ));
}
