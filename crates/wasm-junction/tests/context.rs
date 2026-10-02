//! Call context tests.

use std::sync::Arc;

use wasm_junction::{CallContext, Caller};

#[test]
fn test_context_names_a_component_and_starts_empty() {
    let context = CallContext::for_test("summarizer");

    assert_eq!(
        context.caller(),
        &Caller::Component(Arc::from("summarizer"))
    );
    assert!(context.extensions().get::<String>().is_none());
}

#[test]
fn test_context_carries_attached_data() {
    struct SessionId(u32);

    let context = CallContext::for_test("writer").with(SessionId(42));

    assert_eq!(
        context.extensions().get::<SessionId>().map(|value| value.0),
        Some(42)
    );
}

#[test]
fn test_context_has_optional_type_indexed_settings() {
    #[derive(Clone)]
    struct Notebook(&'static str);

    let empty = CallContext::for_test("writer");
    assert!(empty.settings::<Notebook>().is_none());

    let configured = empty
        .with_setting(Notebook("drafts"))
        .with_setting(Notebook("research"));
    assert_eq!(
        configured.settings::<Notebook>().map(|value| value.0),
        Some("research")
    );
}
