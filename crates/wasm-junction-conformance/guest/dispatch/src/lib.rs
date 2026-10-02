//! Component that repeatedly calls a plain host import.

mod bindings {
    wit_bindgen::generate!({
        path: "../../dispatch-wit",
        world: "example:dispatch/dispatch@0.1.0",
        generate_all,
    });
}

use bindings::example::dispatch::pinger;

struct Component;

impl bindings::exports::example::dispatch::runner::Guest for Component {
    fn echo_bytes(bytes: Vec<u8>) -> Vec<u8> {
        bytes
    }

    fn imports(iterations: u32) -> u32 {
        let mut value = 0;
        for _ in 0..iterations {
            value = pinger::ping(value);
        }
        value
    }

    fn noop() -> u32 {
        0
    }
}

bindings::export!(Component with_types_in bindings);
