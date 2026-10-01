//! Second generation of the reload fixture.

mod bindings {
    wit_bindgen::generate!({ path: "../../reload-wit", world: "greeter-component", generate_all });
}

use bindings::example::reload::gate;

struct Component;

impl bindings::exports::example::reload::greeter::Guest for Component {
    fn greet(name: String) -> String {
        format!("v2: hello, {name}")
    }

    async fn greet_slow(name: String) -> String {
        gate::wait().await;
        Self::greet(name)
    }
}

impl bindings::exports::example::reload::legacy::Guest for Component {
    fn status() -> String {
        "v2 legacy".to_owned()
    }
}

bindings::export!(Component with_types_in bindings);
