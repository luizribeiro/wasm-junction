//! Fake Google translator component.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:translate/translator-component@0.1.0",
        generate_all,
    });
}

struct Component;

impl bindings::exports::example::translate::translator::Guest for Component {
    async fn translate(text: String, target_language: String) -> Result<String, String> {
        match (text.as_str(), target_language.as_str()) {
            ("hello", "pt") => Ok("[Google] oi".to_owned()),
            _ => Err(format!("[Google] no {target_language} translation for {text:?}")),
        }
    }
}

bindings::export!(Component with_types_in bindings);
