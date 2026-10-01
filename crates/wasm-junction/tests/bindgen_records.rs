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
}

#[test]
fn records_encode_to_named_wit_fields() {
    let value = wasm_junction::Val::from(notes::Coordinates { x: 3, y: -2 });
    assert_eq!(
        value,
        wasm_junction::Val::Record(vec![
            ("x".to_owned(), wasm_junction::Val::S32(3)),
            ("y".to_owned(), wasm_junction::Val::S32(-2)),
        ])
    );
}
