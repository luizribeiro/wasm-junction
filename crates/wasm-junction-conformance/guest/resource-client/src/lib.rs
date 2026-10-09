//! Host-resource component used to exercise resource ownership.

mod bindings {
    wit_bindgen::generate!({
        path: "../../resource-wit",
        world: "example:resources/resource-client@1.0.0",
        generate_all,
    });
}

use bindings::example::resources::host::{File, Session};

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

    fn retain() -> String {
        let session = Session::new("Grace");
        let profile = session.profile();
        core::mem::forget(session);
        core::mem::forget(Session::new("cleanup"));
        profile
    }

    fn drop_after_refusal() {
        let session = Session::new("refuse");
        let _ = session.profile();
        drop(session);
    }

    fn inspect(value: &Session) -> String {
        value.profile()
    }

    fn round_trip_file(value: File) -> File {
        value
    }

    fn round_trip(value: Session) -> Session {
        value
    }
    fn return_sessions() -> Vec<Session> {
        vec![Session::new("Ada"), Session::new("Grace")]
    }
}

bindings::export!(Component with_types_in bindings);
