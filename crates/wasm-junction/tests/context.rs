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
