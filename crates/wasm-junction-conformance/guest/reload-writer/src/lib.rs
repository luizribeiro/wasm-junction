//! Routed caller used by the reload fixture.

mod bindings {
    wit_bindgen::generate!({ path: "../../reload-wit", world: "writer-component", generate_all });
}

use bindings::example::reload::greeter;

struct Component;

impl bindings::exports::example::reload::writer::Guest for Component {
    fn write(name: String) -> String {
        greeter::greet(&name)
    }
}

bindings::export!(Component with_types_in bindings);
