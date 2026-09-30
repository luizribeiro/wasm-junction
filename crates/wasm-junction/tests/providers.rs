//! Host provider tests.

mod support;

use std::collections::HashMap;

use support::{NOTES, Read, block_on, read_call};
use wasm_junction::{
    BoxFuture, Call, CallContext, Caller, Provided, Provider, Trap, TypedCall, Vals,
};

struct NotesProvider {
    notes: HashMap<String, String>,
}

impl Provider for NotesProvider {
    fn call<'a>(&'a self, cx: &'a CallContext, call: Call) -> BoxFuture<'a, Result<Vals, Trap>> {
        Box::pin(async move {
            if cx.caller() != &Caller::Component(String::from("summarizer")) {
                return Err(Trap::new("caller may not read notes"));
            }
            let read = call
                .view::<Read>()?
                .ok_or_else(|| Trap::new("notes provider received another function"))?;
            let text = self
                .notes
                .get(&read.name)
                .ok_or_else(|| Trap::new(format!("note `{}` was not found", read.name)))?;
            Ok(Read::output(text.clone()))
        })
    }
}

fn provider() -> NotesProvider {
    NotesProvider {
        notes: HashMap::from([(String::from("daily"), String::from("buy tea"))]),
    }
}

#[test]
fn provider_is_called_with_an_engine_free_context() {
    let provider = provider();
    let context = CallContext::for_test("summarizer");
    let values = block_on(provider.call(&context, read_call("daily"))).unwrap();

    assert_eq!(Read::decode_output(&values).unwrap(), "buy tea");
}

#[test]
fn provider_failures_become_traps() {
    let provider = provider();
    let context = CallContext::for_test("summarizer");
    let error = block_on(provider.call(&context, read_call("missing"))).unwrap_err();

    assert_eq!(error.to_string(), "note `missing` was not found");
}

#[test]
fn provider_future_can_be_dropped_before_polling() {
    let provider = provider();
    let context = CallContext::for_test("summarizer");
    drop(provider.call(&context, read_call("daily")));
}

#[test]
fn generated_constructor_can_make_a_provided_interface() {
    let provided = Provided::new(NOTES, provider());
    assert!(format!("{provided:?}").contains(NOTES));
}
