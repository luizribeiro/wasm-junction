//! Tests for WIT names that are Rust keywords.

#![forbid(unsafe_code)]

wasm_junction::bindgen!({ path: "tests/fixtures/keywords/wit" });

#[test]
fn keywords_generate_valid_documented_identifiers() {
    let value = super_::Self_ {
        r#type: "type".to_owned(),
        r#match: true,
        self_: "self".to_owned(),
        super_: "super".to_owned(),
        crate_: "crate".to_owned(),
    };
    let encoded = wasm_junction::Val::from(value.clone());
    assert_eq!(super_::Self_::try_from(encoded).unwrap(), value);
}
