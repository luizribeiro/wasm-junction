//! Component lifecycle tests using an engine-neutral fake engine.

mod support;

use std::sync::{Arc, Mutex};

use support::{
    FailingEngine, FakeEngine, GenerationEngine, GenerationState, block_on, component_bytes,
    component_bytes_from,
};
use wasm_junction::{
    App, Call, CallContext, CallError, CallErrorKind, Component, Event, Middleware, Next,
    ReloadError, UnloadError, Val, Vals,
};

mod handles {
    wasm_junction::bindgen!({ path: "tests/fixtures/handles/wit" });
}

struct EventLog(Arc<Mutex<Vec<Event>>>);

impl Middleware for EventLog {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        next.run(call).await
    }

    fn event(&self, event: &Event) {
        self.0.lock().unwrap().push(event.clone());
    }
}

const TRANSLATOR: &str = "example:translate/translator@1.0.0";
const TRANSLATOR_WIT: &str = r"
    package example:translate@1.0.0;
    interface translator { translate: func(text: string) -> string; }
    world service { export translator; }
";
const TRANSLATOR_V0_WIT: &str = r"
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
    world writer { import example:translate/translator@0.1.0; export article; }
";
const MARKER_WIT: &str = r"
    package example:marker@1.0.0;
    interface marker { mark: func(); }
    world service { export marker; }
";
const CYCLE_WIT: &str = r"
    package example:cycle@1.0.0;
    interface first-api { ping: func(); }
    world first-component { import second-api; export first-api; }
    interface second-api { ping: func(); }
    world second-component { import first-api; export second-api; }
";

fn component() -> Component {
    Component::from_bytes(component_bytes(TRANSLATOR_WIT, "service")).unwrap()
}

fn writer() -> Component {
    Component::from_bytes(component_bytes_from(
        &[
            ("translator.wit", TRANSLATOR_API_WIT),
            ("writer.wit", WRITER_WIT),
        ],
        "example:writer/writer@1.0.0",
    ))
    .unwrap()
}

#[test]
fn in_flight_calls_pin_the_old_generation_until_they_finish() {
    let state = Arc::new(GenerationState::default());
    let app = App::builder()
        .engine(GenerationEngine(state.clone()))
        .build()
        .unwrap();
    block_on(app.load(component().named("translator"))).unwrap();

    let caller = app.clone();
    let in_flight = std::thread::spawn(move || {
        block_on(caller.call("translator", TRANSLATOR, "translate", Vec::new())).unwrap()
    });
    state.wait_until_called();
    block_on(app.reload("translator", component())).unwrap();
    let old = state.generation(0);
    assert!(old.upgrade().is_some());
    state.release();

    assert_eq!(in_flight.join().unwrap(), [Val::from("old")]);
    assert!(old.upgrade().is_none());
    assert_eq!(
        block_on(app.call("translator", TRANSLATOR, "translate", Vec::new())).unwrap(),
        [Val::from("new")]
    );
    assert!(matches!(
        block_on(app.reload("translator", writer())).unwrap_err(),
        ReloadError::MissingImports(_)
    ));
}

#[test]
fn reload_checks_imports_and_preserves_compatible_links() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(
        app.load(
            Component::from_bytes(component_bytes(TRANSLATOR_V0_WIT, "service"))
                .unwrap()
                .named("one"),
        ),
    )
    .unwrap();
    block_on(app.load(writer().named("writer"))).unwrap();
    app.link("writer", "example:translate/translator@0.1.0", "one")
        .unwrap();
    block_on(
        app.load(
            Component::from_bytes(component_bytes(TRANSLATOR_V0_WIT, "service"))
                .unwrap()
                .named("two"),
        ),
    )
    .unwrap();

    block_on(app.reload("writer", writer())).unwrap();
    assert_eq!(
        block_on(app.call(
            "writer",
            "example:writer/article@1.0.0",
            "write",
            vec![Val::from("hello")],
        ))
        .unwrap(),
        [Val::from("translated: hello")]
    );
}

#[test]
fn reload_refuses_to_make_an_existing_import_ambiguous() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(
        app.load(
            Component::from_bytes(component_bytes(TRANSLATOR_V0_WIT, "service"))
                .unwrap()
                .named("translator"),
        ),
    )
    .unwrap();
    block_on(app.load(writer().named("writer"))).unwrap();
    block_on(
        app.load(
            Component::from_bytes(component_bytes(MARKER_WIT, "service"))
                .unwrap()
                .named("candidate"),
        ),
    )
    .unwrap();

    let error = block_on(app.reload(
        "candidate",
        Component::from_bytes(component_bytes(TRANSLATOR_V0_WIT, "service")).unwrap(),
    ))
    .unwrap_err();
    let ReloadError::WouldMakeAmbiguous { issues } = error else {
        panic!("expected an ambiguity");
    };
    assert_eq!(issues[0].component, "writer");
}

#[test]
fn breaking_reload_names_component_dependents_and_force_is_clear() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(
        app.load(
            Component::from_bytes(component_bytes(TRANSLATOR_V0_WIT, "service"))
                .unwrap()
                .named("translator"),
        ),
    )
    .unwrap();
    block_on(app.load(writer().named("linked-writer"))).unwrap();
    block_on(app.load(writer().named("resolved-writer"))).unwrap();
    app.link(
        "linked-writer",
        "example:translate/translator@0.1.0",
        "translator",
    )
    .unwrap();

    let replacement = || Component::from_bytes(component_bytes(MARKER_WIT, "service")).unwrap();
    let error = block_on(app.reload("translator", replacement())).unwrap_err();
    let ReloadError::Breaking { dependents, .. } = error else {
        panic!("expected a breaking reload");
    };
    assert!(dependents.iter().any(|item| item.contains("linked-writer")));
    assert!(
        dependents
            .iter()
            .any(|item| item.contains("resolved-writer"))
    );

    block_on(app.reload_force("translator", replacement())).unwrap();
    let error = block_on(app.call(
        "linked-writer",
        "example:writer/article@1.0.0",
        "write",
        vec![Val::from("hello")],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Unavailable);
    assert!(error.to_string().contains("translator"));
    assert!(error.to_string().contains("no longer exports"));
}

#[test]
fn derived_typed_handles_share_a_dependency_lease() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    let component = Component::from_bytes(component_bytes(
        include_str!("fixtures/handles/wit/package.wit"),
        "plugin",
    ))
    .unwrap();
    block_on(app.load(component.named("journal"))).unwrap();
    let handle = app.get::<handles::summaries::Summaries>("journal").unwrap();
    let with_data = handle.with(42_u32);
    let context = CallContext::for_test("writer");
    let within_call = handle.within(&context);
    drop(handle);

    let replacement = || Component::from_bytes(component_bytes(MARKER_WIT, "service")).unwrap();
    let error = block_on(app.reload("journal", replacement())).unwrap_err();
    let ReloadError::Breaking { dependents, .. } = error else {
        panic!("expected the handle to prevent a breaking reload");
    };
    assert!(dependents[0].contains("host handle"));
    assert!(matches!(
        block_on(app.unload("journal")).unwrap_err(),
        UnloadError::HasDependents { .. }
    ));

    drop(with_data);
    assert!(matches!(
        block_on(app.reload("journal", replacement())),
        Err(ReloadError::Breaking { .. })
    ));

    drop(within_call);
    block_on(app.reload("journal", replacement())).unwrap();
}

#[test]
fn unload_pins_an_in_flight_call_and_rejects_new_calls() {
    let state = Arc::new(GenerationState::default());
    let app = App::builder()
        .engine(GenerationEngine(state.clone()))
        .build()
        .unwrap();
    block_on(app.load(component().named("translator"))).unwrap();

    let caller = app.clone();
    let in_flight = std::thread::spawn(move || {
        block_on(caller.call("translator", TRANSLATOR, "translate", Vec::new())).unwrap()
    });
    state.wait_until_called();
    block_on(app.unload("translator")).unwrap();
    let old = state.generation(0);
    assert!(old.upgrade().is_some());
    state.release();

    assert_eq!(in_flight.join().unwrap(), [Val::from("old")]);
    assert!(old.upgrade().is_none());
    let error = block_on(app.call("translator", TRANSLATOR, "translate", Vec::new())).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Unavailable);
    assert!(error.to_string().contains("was unloaded"));
}

#[test]
fn forced_unload_names_the_provider_for_linked_and_implicit_imports() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    block_on(
        app.load(
            Component::from_bytes(component_bytes(TRANSLATOR_V0_WIT, "service"))
                .unwrap()
                .named("translator"),
        ),
    )
    .unwrap();
    block_on(app.load(writer().named("linked-writer"))).unwrap();
    block_on(app.load(writer().named("implicit-writer"))).unwrap();
    app.link(
        "linked-writer",
        "example:translate/translator@0.1.0",
        "translator",
    )
    .unwrap();

    let error = block_on(app.unload("translator")).unwrap_err();
    assert!(error.to_string().contains("linked-writer"));
    assert!(error.to_string().contains("implicit-writer"));
    block_on(app.unload_force("translator")).unwrap();
    for writer in ["linked-writer", "implicit-writer"] {
        let error = block_on(app.call(
            writer,
            "example:writer/article@1.0.0",
            "write",
            vec![Val::from("hello")],
        ))
        .unwrap_err();
        assert_eq!(error.kind(), CallErrorKind::Unavailable);
        assert!(error.to_string().contains("translator"));
        assert!(error.to_string().contains("was unloaded"));
    }
}

#[test]
fn reload_all_swaps_interdependent_components_or_changes_nothing() {
    let app = App::builder().engine(FakeEngine).build().unwrap();
    for name in ["first", "second"] {
        block_on(
            app.load(
                Component::from_bytes(component_bytes(MARKER_WIT, "service"))
                    .unwrap()
                    .named(name),
            ),
        )
        .unwrap();
    }
    let cyclic = |world| Component::from_bytes(component_bytes(CYCLE_WIT, world)).unwrap();
    block_on(app.reload_all([
        ("first", cyclic("first-component")),
        ("second", cyclic("second-component")),
    ]))
    .unwrap();
    app.check().unwrap();

    let error = block_on(app.reload_all([
        (
            "missing",
            Component::from_bytes(component_bytes(MARKER_WIT, "service")).unwrap(),
        ),
        (
            "first",
            Component::from_bytes(component_bytes(MARKER_WIT, "service")).unwrap(),
        ),
    ]))
    .unwrap_err();
    assert!(matches!(error, ReloadError::UnknownComponent(_)));
    app.check().unwrap();
}

#[test]
fn reload_all_changes_nothing_when_a_later_compile_fails() {
    struct BatchExport<const MARKER: bool>;

    impl<const MARKER: bool> wasm_junction::InterfaceHandle for BatchExport<MARKER> {
        const INTERFACE: &'static str = if MARKER {
            "example:marker/marker@1.0.0"
        } else {
            TRANSLATOR
        };

        fn from_app(_app: App, _component: Arc<str>) -> Self {
            Self
        }
    }

    let app = App::builder()
        .engine(FailingEngine::after(3))
        .build()
        .unwrap();
    block_on(app.load(component().named("first"))).unwrap();
    block_on(app.load(component().named("second"))).unwrap();

    assert!(matches!(
        block_on(app.reload_all([
            (
                "first",
                Component::from_bytes(component_bytes(MARKER_WIT, "service")).unwrap(),
            ),
            (
                "second",
                Component::from_bytes(component_bytes(MARKER_WIT, "service")).unwrap(),
            ),
        ])),
        Err(ReloadError::Compile(error)) if error.to_string() == "invalid adapter"
    ));
    for name in ["first", "second"] {
        assert!(app.has::<BatchExport<false>>(name));
        assert!(!app.has::<BatchExport<true>>(name));
    }
}

#[test]
fn middleware_observes_load_reload_and_unload_exports() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(FakeEngine)
        .middleware(EventLog(events.clone()))
        .build()
        .unwrap();
    block_on(app.load(component().named("service"))).unwrap();
    block_on(app.reload(
        "service",
        Component::from_bytes(component_bytes(MARKER_WIT, "service")).unwrap(),
    ))
    .unwrap();
    block_on(app.unload("service")).unwrap();

    let events = events.lock().unwrap();
    assert!(matches!(
        events.as_slice(),
        [
            Event::Load { component, exports },
            Event::Reload { component: reloaded, old_exports, new_exports },
            Event::Unload { component: unloaded },
        ] if component.as_ref() == "service"
            && reloaded.as_ref() == "service"
            && unloaded.as_ref() == "service"
            && exports[0].as_ref() == TRANSLATOR
            && old_exports[0].as_ref() == TRANSLATOR
            && new_exports[0].as_ref() == "example:marker/marker@1.0.0"
    ));
}
