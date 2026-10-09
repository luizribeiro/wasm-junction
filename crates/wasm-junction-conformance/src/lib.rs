//! Engine-neutral fixtures shared by wasm-junction engine implementations.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod host;
mod reload;
mod resources;
mod runner;
mod stream_host;
mod trace;
mod value_stream_host;

wasm_junction::bindgen!({ path: "wit" });

mod stream_bindings {
    wasm_junction::bindgen!({ path: "stream-wit", interfaces: ["example:streams/host"] });
}

mod value_stream_bindings {
    wasm_junction::bindgen!({
        path: "value-stream-wit",
        interfaces: ["example:value-streams/host@0.1.0"]
    });
}

pub use host::{FixtureHost, RoutedHost, sample_note, sample_summary};
pub use reload::{ReloadGreeter, ReloadHost};
pub use resources::ResourceHost;
pub use runner::{
    Fixture, FixtureError, ResourceFixture, RoutedFixture, StreamFixture, run, run_default,
    run_reload, run_resource_refusal, run_resources, run_routed, run_streams,
};
pub use stream_host::{PoisonHost, RetainHost, StreamHost};
pub use trace::{
    EXPECTED_RELOAD_TRACE, EXPECTED_RESOURCE_REFUSAL_TRACE, EXPECTED_RESOURCE_TRACE,
    EXPECTED_ROUTED_TRACE, EXPECTED_STREAM_TRACE, EXPECTED_TRACE, Trace,
};
pub use value_stream_host::ValueStreamHost;

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
/// The byte-stream fixture's imported host interface.
pub const STREAM_HOST: &str = "example:streams/host@0.1.0";
/// The byte-stream fixture's exported interface.
pub const STREAM_PROBE: &str = "example:streams/probe@0.1.0";
/// The value-stream fixture's imported host interface.
pub const VALUE_STREAM_HOST: &str = "example:value-streams/host@0.1.0";
/// The value-stream fixture's exported interface.
pub const VALUE_STREAM_PROBE: &str = "example:value-streams/probe@0.1.0";
/// The reload fixture's host gate interface.
pub const RELOAD_GATE: &str = "example:reload/gate@0.1.0";
/// The interface replaced by a breaking reload.
pub const RELOAD_GREETER: &str = "example:reload/greeter@0.1.0";
/// The reload fixture's routed caller interface.
pub const RELOAD_WRITER: &str = "example:reload/writer@0.1.0";
/// The dispatch fixture's imported host interface.
pub const DISPATCH_PINGER: &str = "example:dispatch/pinger@0.1.0";
/// The dispatch fixture's exported loop interface.
pub const DISPATCH_RUNNER: &str = "example:dispatch/runner@0.1.0";

pub(crate) struct SessionId(pub u32);
pub(crate) struct TranslatorHop;
#[derive(Clone)]
pub(crate) struct ComponentSettings(pub &'static str);

/// Returns the notes-summary fixture component.
#[must_use]
pub fn component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/notes-summary.wasm"))
}

/// Returns the fixture that repeatedly calls a plain host import.
#[must_use]
pub fn dispatch_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/dispatch.wasm"))
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

/// Returns the component that exchanges resource streams with its host.
#[must_use]
pub fn resource_stream_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/resource-stream.wasm"))
}

/// Returns the component that exchanges resource streams with its host.
#[must_use]
pub fn invalid_resource_stream_component() -> &'static [u8] {
    resource_stream_component()
}
/// Returns the component that exchanges byte streams with its host.
#[must_use]
pub fn stream_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/streams.wasm"))
}

/// Returns the component that exchanges value streams with its host.
#[must_use]
pub fn value_stream_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/value-streams.wasm"))
}

/// Returns the first reload fixture generation.
#[must_use]
pub fn reload_v1_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/reload-v1.wasm"))
}

/// Returns the second reload fixture generation.
#[must_use]
pub fn reload_v2_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/reload-v2.wasm"))
}

/// Returns a reload fixture generation without the greeter interface.
#[must_use]
pub fn reload_breaking_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/reload-breaking.wasm"))
}

/// Returns the reload fixture's routed caller.
#[must_use]
pub fn reload_writer_component() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/reload-writer.wasm"))
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
    fn dispatch_component_has_one_import_and_one_export() {
        let component = Component::from_bytes(dispatch_component()).unwrap();
        assert_eq!(component.imports(), [DISPATCH_PINGER]);
        assert_eq!(component.exports(), [DISPATCH_RUNNER]);
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

    #[test]
    fn resource_stream_component_uses_the_resource_interfaces() {
        let component = Component::from_bytes(resource_stream_component()).unwrap();
        assert_eq!(component.imports(), [RESOURCE_HOST]);
        assert_eq!(component.exports(), [RESOURCE_CLIENT]);
    }

    #[test]
    fn invalid_resource_stream_component_uses_the_resource_interfaces() {
        let component = Component::from_bytes(invalid_resource_stream_component()).unwrap();
        assert_eq!(component.imports(), [RESOURCE_HOST]);
        assert_eq!(component.exports(), [RESOURCE_CLIENT]);
    }

    #[test]
    fn stream_component_has_complementary_interfaces() {
        let component = Component::from_bytes(stream_component()).unwrap();
        assert_eq!(component.imports(), [STREAM_HOST]);
        assert_eq!(component.exports(), [STREAM_PROBE]);
    }

    #[test]
    fn value_stream_component_has_complementary_interfaces() {
        let component = Component::from_bytes(value_stream_component()).unwrap();
        assert_eq!(component.imports(), [VALUE_STREAM_HOST]);
        assert_eq!(component.exports(), [VALUE_STREAM_PROBE]);
    }
}
