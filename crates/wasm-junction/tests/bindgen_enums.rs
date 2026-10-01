//! Tests for generated WIT enums.

#![forbid(unsafe_code)]

wasm_junction::bindgen!({ path: "tests/fixtures/shapes/wit" });

#[test]
fn every_enum_case_round_trips() {
    for status in [controls::Status::Ready, controls::Status::NeedsReview] {
        let value = wasm_junction::Val::from(status);
        assert_eq!(controls::Status::try_from(value).unwrap(), status);
    }
    assert_eq!(
        wasm_junction::Val::from(controls::Status::NeedsReview),
        wasm_junction::Val::Enum("needs-review".to_owned())
    );
    assert!(controls::Status::try_from(wasm_junction::Val::Enum("missing".to_owned())).is_err());
}
