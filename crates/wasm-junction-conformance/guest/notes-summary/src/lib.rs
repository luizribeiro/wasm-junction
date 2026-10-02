//! Notes-summary component used to exercise engine boundaries.

mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "example:notes/notes-summary@0.1.0",
        generate_all,
    });
}

use bindings::example::notes::{notes, types};

struct Component;

impl bindings::exports::example::notes::summarizer::Guest for Component {
    async fn summarize(name: String) -> Result<types::Summary, String> {
        let note = notes::read(name).await?;
        let note = notes::normalize(&note);
        Ok(types::Summary {
            text: format!("{}: {} tags", note.title, note.tags.len()),
            source: note,
        })
    }

    async fn repeat_after_refusal() {
        loop {
            let _ = notes::read("provider-refusal".to_owned()).await;
        }
    }

    fn echo(value: types::Note) -> types::Note {
        notes::normalize(&value)
    }

    fn crash() {
        panic!("fixture trap")
    }
}

bindings::export!(Component with_types_in bindings);
