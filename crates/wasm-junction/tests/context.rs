//! Call context tests.

use wasm_junction::{CallContext, Caller};

#[test]
fn test_context_names_a_component_and_starts_empty() {
    let context = CallContext::for_test("summarizer");

    assert_eq!(
        context.caller(),
        &Caller::Component(String::from("summarizer"))
    );
    assert!(context.extensions().get::<String>().is_none());
}
