//! Tests for generated WIT enums.

#![forbid(unsafe_code)]

wasm_junction::bindgen!({ path: "tests/fixtures/shapes/wit" });

fn requires_error<T: std::error::Error>() {}

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

#[test]
fn every_variant_case_round_trips() {
    for choice in [
        controls::Choice::Text("draft".to_owned()),
        controls::Choice::Number(4),
        controls::Choice::None,
    ] {
        let value = wasm_junction::Val::from(choice.clone());
        assert_eq!(controls::Choice::try_from(value).unwrap(), choice);
    }
    let wrong = wasm_junction::Val::Variant {
        case: "number".to_owned(),
        value: Some(Box::new(wasm_junction::Val::String("four".to_owned()))),
    };
    assert!(controls::Choice::try_from(wrong).is_err());
}
#[test]
fn primitives_and_nested_shapes_round_trip() {
    let scalars = controls::ScalarValues {
        boolean: true,
        unsigned_eight: 8,
        unsigned_sixteen: 16,
        unsigned_thirty_two: 32,
        unsigned_sixty_four: 64,
        signed_eight: -8,
        signed_sixteen: -16,
        signed_thirty_two: -32,
        signed_sixty_four: -64,
        float_thirty_two: 32.5,
        float_sixty_four: 64.5,
        character: '🦀',
        text: "all scalars".to_owned(),
    };
    let nested = controls::NestedValues {
        items: vec![scalars],
        maybe_status: Some(controls::Status::Ready),
        pair: (9, "tuple".to_owned()),
        outcome: Ok(controls::Choice::Number(4)),
        success_only: Ok(7),
        failure_only: Err("denied".to_owned()),
    };
    let value = wasm_junction::Val::from(nested.clone());
    assert_eq!(controls::NestedValues::try_from(value).unwrap(), nested);
}

#[test]
fn result_error_types_implement_display_and_error() {
    requires_error::<controls::Problem>();
    assert_eq!(controls::Problem::Denied.to_string(), "denied");
    let value = wasm_junction::Val::from(controls::Problem::Invalid);
    assert_eq!(
        controls::Problem::try_from(value).unwrap(),
        controls::Problem::Invalid
    );
}

#[test]
fn variant_errors_display_as_messages() {
    requires_error::<controls::Failure>();
    assert_eq!(controls::Failure::NotFound.to_string(), "not-found");
    assert_eq!(
        controls::Failure::Message("bad input".into()).to_string(),
        "message: bad input"
    );
    assert_eq!(
        controls::Failure::RetryAfter(5).to_string(),
        "retry-after: 5"
    );
    assert_eq!(
        controls::Failure::Structured(controls::Choice::None).to_string(),
        "structured: None"
    );
    assert_eq!(
        controls::Failure::Nested(controls::Problem::Denied).to_string(),
        "nested: denied"
    );
}

#[test]
fn record_errors_display_as_field_messages() {
    requires_error::<controls::Details>();
    let error = controls::Details {
        message: "bad input".into(),
        code: 7,
        choice: controls::Choice::None,
        problem: controls::Problem::Invalid,
    };
    assert_eq!(
        error.to_string(),
        "message: bad input, code: 7, choice: None, problem: invalid"
    );
}
