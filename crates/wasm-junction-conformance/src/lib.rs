//! Engine-neutral fixtures shared by wasm-junction engine implementations.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod host;
mod runner;
mod trace;

wasm_junction::bindgen!({ path: "wit" });

pub use host::{FixtureHost, sample_note, sample_summary};
pub use runner::{Fixture, FixtureError, run};
pub use trace::{EXPECTED_TRACE, Trace};

/// The fixture's types-only interface.
pub const TYPES: &str = types::INTERFACE;
/// The fixture's imported host interface.
pub const NOTES: &str = notes::INTERFACE;
/// The fixture's exported plugin interface.
pub const SUMMARIZER: &str = summarizer::INTERFACE;

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
