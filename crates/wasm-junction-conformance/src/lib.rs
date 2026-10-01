//! Engine-neutral fixtures shared by wasm-junction engine implementations.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod host;
mod runner;
mod trace;

wasm_junction::bindgen!({ path: "wit" });

pub use host::{FixtureHost, RoutedHost, sample_note, sample_summary};
pub use runner::{Fixture, FixtureError, run};
pub use trace::{EXPECTED_TRACE, Trace};

/// The fixture's types-only interface.
pub const TYPES: &str = types::INTERFACE;
/// The fixture's imported host interface.
pub const NOTES: &str = notes::INTERFACE;
/// The fixture's exported plugin interface.
pub const SUMMARIZER: &str = summarizer::INTERFACE;
/// The routed fixture's imported host interface.
pub const DECORATION: &str = decoration::INTERFACE;
/// The interface exported by the routed fixture's translator.
pub const TRANSLATOR: &str = translator::INTERFACE;
/// The interface exported by the routed fixture's writer.
pub const WRITER: &str = writer::INTERFACE;

/// Returns the notes-summary fixture component.
#[must_use]
pub fn component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/notes-summary.wasm"))
}

/// Returns the routed fixture's translator component.
#[must_use]
pub fn translator_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/translator.wasm"))
}

/// Returns the routed fixture's writer component.
#[must_use]
pub fn writer_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/writer.wasm"))
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

    #[test]
    fn routed_components_have_complementary_interfaces() {
        let translator = Component::from_bytes(translator_component()).unwrap();
        assert_eq!(translator.imports(), [DECORATION]);
        assert_eq!(translator.exports(), [TRANSLATOR]);
        let writer = Component::from_bytes(writer_component()).unwrap();
        assert_eq!(writer.imports(), [TRANSLATOR]);
        assert_eq!(writer.exports(), [WRITER]);
    }
}
