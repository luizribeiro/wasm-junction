//! Guest component that greets users returned by the host.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:greeter/plugin@0.1.0",
        generate_all,
    });
}

use bindings::example::greeter::users;

struct Component;

impl bindings::exports::example::greeter::greeter::Guest for Component {
    async fn greet(id: u32) -> Result<String, String> {
        let Some(user) = users::lookup(id) else {
            return Err(format!("user {id} not found"));
        };
        let greeting = match user.language.as_str() {
            "pt" => "Olá",
            "es" => "Hola",
            _ => "Hello",
        };
        Ok(format!("{greeting}, {}!", user.name))
    }
}

bindings::export!(Component with_types_in bindings);
