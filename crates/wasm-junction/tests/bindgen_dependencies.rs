//! Tests for types referenced from dependency packages.

#![forbid(unsafe_code)]

wasm_junction::bindgen!({ path: "tests/fixtures/dependencies/wit" });

#[test]
fn dependency_types_and_uses_share_one_generated_type() {
    let author = types::Author {
        name: "Ada".to_owned(),
    };
    let note = notes::Note {
        author: author.clone(),
    };
    let used: notes::Author = author;

    assert_eq!(
        notes::Note::try_from(wasm_junction::Val::from(note.clone())).unwrap(),
        note
    );
    assert_eq!(used.name, "Ada");
}
