//! Guest component that sends an incrementally produced audit log to its host.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:audit/plugin@0.1.0",
        generate_all,
    });
}

use bindings::example::audit::audit::{self, Session};

struct Component;

impl bindings::exports::example::audit::runner::Guest for Component {
    async fn run(user: String) {
        let session = Session::new(&user);
        let user = session.user();
        let (mut writer, reader) = bindings::wit_stream::new();
        wit_bindgen::spawn_local(async move {
            for line in [
                format!("session opened for {user}\n"),
                "viewed dashboard\n".to_owned(),
                "session complete\n".to_owned(),
            ] {
                let _ = writer.write_all(line.into_bytes()).await;
            }
        });
        audit::audit(reader).await;
        drop(session);
    }
}

bindings::export!(Component with_types_in bindings);
