//! Engine-neutral fixtures shared by wasm-junction engine implementations.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod host;
mod runner;
mod trace;

wasm_junction::bindgen!({ path: "wit" });

pub use host::{FixtureHost, RoutedHost, sample_note, sample_summary};
pub use runner::{Fixture, FixtureError, RoutedFixture, run, run_routed};
pub use trace::{EXPECTED_ROUTED_TRACE, EXPECTED_TRACE, Trace};

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
/// The first exported interface in the cyclic fixture.
pub const CYCLE_A: &str = cycle_a::INTERFACE;
/// The second exported interface in the cyclic fixture.
pub const CYCLE_B: &str = cycle_b::INTERFACE;
/// The host-resource fixture's imported interface.
pub const RESOURCE_HOST: &str = "example:resources/host@1.0.0";
/// The host-resource fixture's exported interface.
pub const RESOURCE_CLIENT: &str = "example:resources/client@1.0.0";

pub(crate) struct SessionId(pub u32);
pub(crate) struct TranslatorHop;

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

/// Returns the first component in the cyclic routed-call fixture.
#[must_use]
pub fn cycle_a_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/cycle-a.wasm"))
}

/// Returns the second component in the cyclic routed-call fixture.
#[must_use]
pub fn cycle_b_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/cycle-b.wasm"))
}

/// Returns the component that imports and uses a host resource.
#[must_use]
pub fn resource_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/resource-client.wasm"))
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

    #[test]
    fn cyclic_components_import_each_other() {
        let a = Component::from_bytes(cycle_a_component()).unwrap();
        assert_eq!(a.imports(), [CYCLE_B]);
        assert_eq!(a.exports(), [CYCLE_A]);
        let b = Component::from_bytes(cycle_b_component()).unwrap();
        assert_eq!(b.imports(), [CYCLE_A]);
        assert_eq!(b.exports(), [CYCLE_B]);
    }

    #[test]
    fn resource_component_imports_only_the_host_resource() {
        let component = Component::from_bytes(resource_component()).unwrap();
        assert_eq!(component.imports(), [RESOURCE_HOST]);
        assert_eq!(component.exports(), [RESOURCE_CLIENT]);
    }
}
