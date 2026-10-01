//! Translator component used to exercise calls back into the host.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:notes/translator-component@0.1.0",
        generate_all,
    });
}

use std::sync::atomic::{AtomicU32, Ordering};

use bindings::example::notes::decoration;

static CALLS: AtomicU32 = AtomicU32::new(0);

struct Component;

impl bindings::exports::example::notes::translator::Guest for Component {
    fn translate(text: String) -> String {
        translated(decoration::decorate(&text))
    }

    async fn translate_async(text: String) -> String {
        translated(decoration::decorate_async(text).await)
    }
}

fn translated(text: String) -> String {
    let call = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    format!("{text} #{call}")
}

bindings::export!(Component with_types_in bindings);
