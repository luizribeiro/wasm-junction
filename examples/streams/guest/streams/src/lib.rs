//! Support-ticket component that exchanges streams with its host.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:streams/plugin@0.1.0",
        generate_all,
    });
}

use bindings::example::streams::support;

struct Component;

impl bindings::exports::example::streams::runner::Guest for Component {
    async fn run() -> String {
        let (mut writer, reader) = bindings::wit_stream::new();
        wit_bindgen::spawn_local(async move {
            for chunk in [
                b"Customer email: ada@exa".to_vec(),
                b"mple.com\nIssue: cannot sign in\n".to_vec(),
            ] {
                let _ = writer.write_all(chunk).await;
            }
        });
        support::store_transcript(reader).await;

        let tickets = support::tickets().collect().await;
        let subjects: Vec<_> = tickets
            .iter()
            .map(|ticket| ticket.strip_prefix("public: ").unwrap_or(ticket))
            .collect();
        format!(
            "{} visible tickets: {}",
            subjects.len(),
            subjects.join("; ")
        )
    }
}

bindings::export!(Component with_types_in bindings);
