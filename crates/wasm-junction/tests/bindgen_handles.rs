//! Tests for generated component handles.

#![forbid(unsafe_code)]

mod support;

use std::sync::Arc;

use wasm_junction::{App, CallContext, CallErrorKind, Component, InterfaceHandle};

wasm_junction::bindgen!({ path: "tests/fixtures/handles/wit" });

#[test]
fn handles_encode_borrowed_arguments_and_decode_every_result_shape() {
    let app = App::builder().engine(support::FakeEngine).build().unwrap();
    let component = Component::from_bytes(support::component_bytes(
        include_str!("fixtures/handles/wit/package.wit"),
        "plugin",
    ))
    .unwrap()
    .named("journal");
    support::block_on(app.load(component)).unwrap();

    let handle = app.get::<summaries::Summaries>("journal").unwrap();
    let attached = handle
        .with(support::ContextMarker(1))
        .with(support::ContextMarker(42));
    assert_eq!(support::block_on(attached.context()).unwrap(), 42);
    assert_eq!(support::block_on(handle.context()).unwrap(), 0);
    let context = CallContext::for_test("writer").with(support::ContextMarker(7));
    assert_eq!(
        support::block_on(handle.within(&context).context()).unwrap(),
        7
    );
    let note = summaries::Note {
        title: "project".into(),
    };
    let selection = summaries::Selection::One("open".into());
    assert_eq!(
        support::block_on(handle.summarize("today", &note, &selection, &[2, 4], 10)).unwrap(),
        "summary"
    );
    assert_eq!(
        support::block_on(handle.accepted()).unwrap(),
        Ok("saved".into())
    );
    assert_eq!(
        support::block_on(handle.rejected()).unwrap(),
        Err("denied".into())
    );
    assert_eq!(
        support::block_on(handle.f(Some("hi"))).unwrap(),
        Some("hi".into())
    );
    assert_eq!(support::block_on(handle.f(None)).unwrap(), None);
    let strings = vec!["a".to_owned(), "b".to_owned()];
    let strs = vec!["a", "b"];
    for joined in [
        support::block_on(handle.g(&["a", "b"])).unwrap(),
        support::block_on(handle.g(&strings)).unwrap(),
        support::block_on(handle.g(&strs)).unwrap(),
    ] {
        assert_eq!(joined, strings);
    }
    let outcome = Ok("done".to_owned());
    let pair = ("pair".to_owned(), 7);
    support::block_on(handle.inspect(&outcome, &pair)).unwrap();
    let handle = handle.clone();
    assert_eq!(support::block_on(handle.clone_("copy")).unwrap(), "copy");
    assert_eq!(
        support::block_on(handle.from_app_("constructor")).unwrap(),
        "constructor"
    );
    assert_eq!(support::block_on(handle.with_("value")).unwrap(), "value");
    assert_eq!(support::block_on(handle.within_("scope")).unwrap(), "scope");

    let missing = summaries::Summaries::from_app(app, Arc::from("missing"));
    let error = support::block_on(missing.accepted()).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Unavailable);
}
