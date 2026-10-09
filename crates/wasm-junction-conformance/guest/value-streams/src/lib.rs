//! Component used to exercise value streams in both directions.

mod bindings {
    wit_bindgen::generate!({
        path: "../../value-stream-wit",
        world: "example:value-streams/value-streams@0.1.0",
        generate_all,
    });
}

use bindings::example::value_streams::host;
use wit_bindgen::StreamReader;

struct Component;

impl bindings::exports::example::value_streams::probe::Guest for Component {
    async fn exchange_strings() -> Vec<String> {
        let (mut writer, reader) = bindings::wit_stream::new();
        wit_bindgen::spawn_local(async move {
            let _ = writer
                .write_all(vec!["guest one".to_owned(), "guest two".to_owned()])
                .await;
        });
        host::accept_strings(reader).await;
        host::strings().collect().await
    }

    async fn echo_strings(values: StreamReader<String>) -> Vec<String> {
        values.collect().await
    }

    fn return_host_strings() -> StreamReader<String> {
        host::strings()
    }

}

bindings::export!(Component with_types_in bindings);
