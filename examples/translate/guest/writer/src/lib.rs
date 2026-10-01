//! Writer component that delegates translation to its import.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:translate/writer-component@0.1.0",
        generate_all,
    });
}

use bindings::example::translate::translator;

struct Component;

impl bindings::exports::example::translate::writer::Guest for Component {
    async fn write(text: String, target_language: String) -> Result<String, String> {
        let translated = translator::translate(text, target_language).await?;
        Ok(format!("Draft: {translated}"))
    }
}

bindings::export!(Component with_types_in bindings);
