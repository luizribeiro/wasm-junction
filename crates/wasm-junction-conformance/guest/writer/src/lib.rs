//! Writer component used to exercise calls between components.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:notes/writer-component@0.1.0",
        generate_all,
    });
}

use bindings::example::notes::translator;

struct Component;

impl bindings::exports::example::notes::writer::Guest for Component {
    fn write(text: String) -> String {
        translator::translate(&text)
    }

    async fn write_async(text: String) -> String {
        translator::translate_async(text).await
    }
}

bindings::export!(Component with_types_in bindings);
