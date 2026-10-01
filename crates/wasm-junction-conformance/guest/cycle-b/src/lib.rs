//! Second half of a component call cycle.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:notes/cycle-b-component@0.1.0",
        generate_all,
    });
}

use bindings::example::notes::cycle_a;

struct Component;

impl bindings::exports::example::notes::cycle_b::Guest for Component {
    fn recurse(depth: u32) -> u32 {
        cycle_a::recurse(depth + 1)
    }
}

bindings::export!(Component with_types_in bindings);
