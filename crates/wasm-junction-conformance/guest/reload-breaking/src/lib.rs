//! Reload fixture generation with a removed interface.

mod bindings {
    wit_bindgen::generate!({ path: "../../reload-wit", world: "breaking-component", generate_all });
}

struct Component;

impl bindings::exports::example::reload::legacy::Guest for Component {
    fn status() -> String {
        "breaking legacy".to_owned()
    }
}

bindings::export!(Component with_types_in bindings);
