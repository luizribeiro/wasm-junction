//! Comparison between generated bindings and the example's hand-written reference.

#![forbid(unsafe_code)]

#[allow(
    dead_code,
    reason = "the reference includes the later typed-handle surface"
)]
#[path = "../src/bindings.rs"]
mod handwritten;

mod generated {
    wasm_junction::bindgen!({ path: "wit" });
}

use wasm_junction::{App, CallContext, Component, TypedCall};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/greeter.wasm"));

struct Directory;

impl generated::users::Host for Directory {
    fn lookup(&self, _cx: &CallContext, id: u32) -> Option<generated::users::User> {
        (id == 42).then(|| generated::users::User {
            name: "Ada".into(),
            language: "en".into(),
        })
    }
}

impl handwritten::users::Host for Directory {
    fn lookup(&self, _cx: &CallContext, id: u32) -> Option<handwritten::users::User> {
        (id == 42).then(|| handwritten::users::User {
            name: "Ada".into(),
            language: "en".into(),
        })
    }
}

#[test]
fn generated_users_match_the_hand_written_value_shape() {
    assert_eq!(generated::users::INTERFACE, handwritten::users::INTERFACE);
    assert_eq!(
        generated::users::Lookup { id: 42 }.into_vals(),
        handwritten::users::Lookup { id: 42 }.into_vals()
    );

    let generated = generated::users::Lookup::output(Some(generated::users::User {
        name: "Ada".into(),
        language: "en".into(),
    }));
    let handwritten = handwritten::users::Lookup::output(Some(handwritten::users::User {
        name: "Ada".into(),
        language: "en".into(),
    }));
    assert_eq!(generated, handwritten);

    let context = CallContext::for_test("greeter");
    let generated = generated::users::Host::lookup(&Directory, &context, 42);
    let handwritten = handwritten::users::Host::lookup(&Directory, &context, 42);
    assert_eq!(
        generated::users::Lookup::output(generated),
        handwritten::users::Lookup::output(handwritten)
    );
}

#[test]
fn generated_async_views_match_the_hand_written_value_shape() {
    assert_eq!(
        generated::greeter::INTERFACE,
        handwritten::greeter::INTERFACE
    );
    assert_eq!(
        generated::greeter::Greet { id: 7 }.into_vals(),
        handwritten::greeter::Greet { id: 7 }.into_vals()
    );
    assert_eq!(
        generated::greeter::Greet::output(Err("missing".into())),
        handwritten::greeter::Greet::output(Err("missing".into()))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn generated_handle_calls_a_wasmtime_component() {
    let app = App::builder()
        .provide(generated::users::provider(Directory))
        .build()
        .unwrap();
    app.load(Component::from_bytes(COMPONENT).unwrap().named("greeter"))
        .await
        .unwrap();

    let greeter = app
        .get::<generated::greeter::Greeter>("greeter")
        .unwrap()
        .clone();
    assert_eq!(
        greeter.greet(42).await.unwrap(),
        Ok("Hello, Ada!".to_owned())
    );
    assert_eq!(app.all::<generated::greeter::Greeter>().len(), 1);
}
