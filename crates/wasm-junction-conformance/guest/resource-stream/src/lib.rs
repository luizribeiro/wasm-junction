//! Component used to exercise resource streams in both directions.

mod bindings {
    wit_bindgen::generate!({
        path: "../../resource-stream-wit",
        world: "example:resources/resource-stream@1.0.0",
        generate_all,
    });
}

use bindings::example::resources::host::{self, Session};
use wit_bindgen::StreamReader;

struct Component;

impl bindings::exports::example::resources::client::Guest for Component {
    fn run(trap: bool) -> String {
        let session = Session::new("Ada");
        let profile = session.profile();
        assert!(!trap, "resource fixture trap");
        profile
    }

    async fn use_host_sessions() -> Vec<String> {
        profiles(
            host::sessions(&["Ada".to_owned(), "Grace".to_owned()])
                .collect()
                .await,
        )
    }

    async fn use_sessions(values: StreamReader<Session>) -> Vec<String> {
        profiles(values.collect().await)
    }

    async fn send_invalid_sessions() -> String {
        let (mut writer, reader) = bindings::wit_stream::new();
        wit_bindgen::spawn_local(async move {
            let _ = writer
                .write_all(vec![Session::new("Ada"), Session::new("duplicate")])
                .await;
        });
        host::accept_sessions(reader).await
    }
}

fn profiles(values: Vec<Session>) -> Vec<String> {
    values
        .into_iter()
        .map(|session| session.profile())
        .collect()
}

bindings::export!(Component with_types_in bindings);
