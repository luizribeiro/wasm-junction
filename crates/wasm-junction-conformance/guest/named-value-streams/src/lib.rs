//! Component used to exercise streams of named values.

mod bindings {
    wit_bindgen::generate!({
        path: "../../named-value-stream-wit",
        world: "example:named-value-streams/named-value-streams@0.1.0",
        generate_all,
    });
}

use bindings::exports::example::named_value_streams::probe::Note;
use wit_bindgen::StreamReader;

struct Component;

impl bindings::exports::example::named_value_streams::probe::Guest for Component {
    async fn echo(values: StreamReader<Note>) -> Vec<Note> {
        values.collect().await
    }
}

bindings::export!(Component with_types_in bindings);
