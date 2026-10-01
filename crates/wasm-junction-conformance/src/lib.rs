//! Engine-neutral fixtures shared by wasm-junction engine implementations.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod host;
mod trace;

pub use host::{FixtureHost, sample_note, sample_summary};
pub use trace::Trace;

/// The fixture's types-only interface.
pub const TYPES: &str = "example:notes/types@0.1.0";
/// The fixture's imported host interface.
pub const NOTES: &str = "example:notes/notes@0.1.0";
/// The fixture's exported plugin interface.
pub const SUMMARIZER: &str = "example:notes/summarizer@0.1.0";

/// Returns the notes-summary fixture component.
#[must_use]
pub fn component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/notes-summary.wasm"))
}

#[cfg(test)]
mod tests {
    use wasm_junction::Component;

    use super::*;

    #[test]
    fn component_has_only_declared_component_imports() {
        let component = Component::from_bytes(component()).unwrap();
        assert_eq!(component.imports(), [NOTES]);
        assert_eq!(component.type_imports(), [TYPES]);
        assert_eq!(component.exports(), [SUMMARIZER]);
    }
}
