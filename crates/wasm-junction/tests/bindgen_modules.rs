//! Tests for package discovery and interface module generation.

#![forbid(unsafe_code)]

wasm_junction::bindgen!({ path: "tests/fixtures/modules/wit" });

#[test]
fn generates_every_local_interface() {
    assert_eq!(note_store::INTERFACE, "test:names/note-store@1.2.3");
    assert_eq!(search::INTERFACE, "test:names/search@1.2.3");
}
