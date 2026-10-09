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

}

fn profiles(values: Vec<Session>) -> Vec<String> {
    values
        .into_iter()
        .map(|session| session.profile())
        .collect()
}

bindings::export!(Component with_types_in bindings);
