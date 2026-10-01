//! Guest component used to measure import dispatch.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "benchmark:dispatch/dispatch@0.1.0",
        generate_all,
    });
}

use bindings::benchmark::dispatch::host;

struct Component;

impl bindings::exports::benchmark::dispatch::runner::Guest for Component {
    fn imports(iterations: u32) -> u32 {
        let mut value = 0;
        for _ in 0..iterations {
            value = host::ping(value);
        }
        value
    }

    fn noop() -> u32 {
        0
    }
}

bindings::export!(Component with_types_in bindings);
