//! Application loading tests.

mod support;

use support::{FakeEngine, NOTES, UnusedProvider, block_on, component_bytes};
use wasm_junction::{App, Component, GetError, InterfaceHandle, LoadError, Provided};

const CLOCK: &str = "example:journal/clock@0.1.0";
const SUMMARIES: &str = "example:journal/summaries@0.1.0";
const PLUGIN_WIT: &str = r"
    package example:journal@0.1.0;
    interface notes { read: func(name: string) -> string; }
    interface clock { now: func() -> u64; }
    interface summaries { summarize: func(note: string) -> string; }
    world plugin { import notes; import clock; export summaries; }
";

fn component(name: &str) -> Component {
    Component::from_bytes(component_bytes(PLUGIN_WIT, "plugin"))
        .unwrap()
        .named(name)
}

struct Summaries;

impl InterfaceHandle for Summaries {
    const INTERFACE: &'static str = SUMMARIES;

    fn from_app(_app: App, _component: String) -> Self {
        Self
    }
}

struct Notes;

impl InterfaceHandle for Notes {
    const INTERFACE: &'static str = NOTES;

    fn from_app(_app: App, _component: String) -> Self {
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
