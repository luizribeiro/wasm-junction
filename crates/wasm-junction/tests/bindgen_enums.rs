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

#[test]
fn flags_round_trip_none_some_and_all() {
    for permissions in [
        controls::Permissions::default(),
        controls::Permissions {
            read: true,
            write: false,
        },
        controls::Permissions {
            read: true,
            write: true,
        },
    ] {
        let value = wasm_junction::Val::from(permissions);
        assert_eq!(controls::Permissions::try_from(value).unwrap(), permissions);
    }
    let invalid = wasm_junction::Val::Flags(vec!["execute".to_owned()]);
    assert!(controls::Permissions::try_from(invalid).is_err());
    let duplicate = wasm_junction::Val::Flags(vec!["read".to_owned(), "read".to_owned()]);
    assert!(controls::Permissions::try_from(duplicate).is_err());
}
