//! Host-resource component used to exercise resource ownership.

mod bindings {
    wit_bindgen::generate!({
        path: "../../resource-wit",
        world: "example:resources/resource-client@1.0.0",
        generate_all,
    });
}

use bindings::example::resources::host::Session;

struct Component;

impl bindings::exports::example::resources::client::Guest for Component {
    fn run(trap: bool) -> String {
        let session = Session::new("Ada");
        let profile = session.profile();
        if trap {
            core::mem::forget(Session::new("cleanup"));
        }
        assert!(!trap, "resource fixture trap");
        profile
    }
}

bindings::export!(Component with_types_in bindings);
