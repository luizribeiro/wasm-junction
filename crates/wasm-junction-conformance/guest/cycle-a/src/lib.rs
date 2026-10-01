//! First half of a component call cycle.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:notes/cycle-a-component@0.1.0",
        generate_all,
    });
}

use bindings::example::notes::cycle_b;

struct Component;

impl bindings::exports::example::notes::cycle_a::Guest for Component {
    fn recurse(depth: u32) -> u32 {
        cycle_b::recurse(depth + 1)
    }
}

bindings::export!(Component with_types_in bindings);
