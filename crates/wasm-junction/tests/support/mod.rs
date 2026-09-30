//! Shared handwritten bindings for integration tests.

use wasm_junction::{Call, Caller};

/// The interface implemented by the handwritten notes fixtures.
pub const NOTES: &str = "example:journal/notes@0.1.0";

/// Builds a notes `read` invocation from the summarizer component.
pub fn read_call(name: &str) -> Call {
    Call::new(
        Caller::Component(String::from("summarizer")),
        "notebook",
        NOTES,
        "read",
        vec![name.into()],
    )
}
