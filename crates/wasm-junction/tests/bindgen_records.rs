//! Tests for generated record definitions.

#![forbid(unsafe_code)]

use std::hash::Hash;

wasm_junction::bindgen!({ path: "tests/fixtures/records/wit" });

fn requires_eq_and_hash<T: Eq + Hash>() {}

#[test]
fn records_have_rust_names_and_expected_derives() {
    requires_eq_and_hash::<notes::Coordinates>();
    let note = notes::Note {
        title: "Plans".to_owned(),
        lines: vec!["Build it".to_owned()],
        marker: Some('!'),
        score: 0.75,
        position: notes::Coordinates { x: 3, y: -2 },
    };
    assert_eq!(note.clone(), note);
    assert!(format!("{note:?}").contains("Plans"));
    let value = wasm_junction::Val::from(note.clone());
    assert_eq!(notes::Note::try_from(value).unwrap(), note);
}

#[test]
fn records_encode_to_named_wit_fields() {
    let coordinates = notes::Coordinates { x: 3, y: -2 };
    let value = wasm_junction::Val::from(coordinates.clone());
    assert_eq!(
        value,
        wasm_junction::Val::Record(vec![
            ("x".to_owned(), wasm_junction::Val::S32(3)),
            ("y".to_owned(), wasm_junction::Val::S32(-2)),
        ])
    );
    assert_eq!(notes::Coordinates::try_from(value).unwrap(), coordinates);
    assert!(notes::Coordinates::try_from(wasm_junction::Val::Record(vec![])).is_err());
}

#[test]
fn record_field_names_do_not_shadow_decoder_temporaries() {
    let value = notes::ValueFirst {
        value: "v".into(),
        tail: 1,
    };
    assert_eq!(
        notes::ValueFirst::try_from(wasm_junction::Val::from(value.clone())).unwrap(),
        value
    );
    let value = notes::FieldFirst {
        field: "f".into(),
        tail: 2,
    };
    assert_eq!(
        notes::FieldFirst::try_from(wasm_junction::Val::from(value.clone())).unwrap(),
        value
    );
    let value = notes::FieldsFirst {
        fields: "fs".into(),
        tail: 3,
    };
    assert_eq!(
        notes::FieldsFirst::try_from(wasm_junction::Val::from(value.clone())).unwrap(),
        value
    );
}
