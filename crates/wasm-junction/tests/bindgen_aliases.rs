//! Tests for generated aliases and cross-interface uses.

#![forbid(unsafe_code)]

wasm_junction::bindgen!({ path: "tests/fixtures/aliases/wit" });

#[test]
fn aliases_follow_rust_naming_and_wit_shapes() {
    let labels: aliases::Labels = vec!["first".to_owned(), "second".to_owned()];
    let pair: aliases::CharacterPair = ('x', true);
    let outcome: aliases::Outcome = Ok(7);
    let copied: aliases::CountCopy = 2;
    let imported: aliases::ImportedCount = copied;

    assert_eq!(labels.len(), 2);
    assert_eq!(pair, ('x', true));
    assert_eq!(outcome, Ok(7));
    assert_eq!(imported, 2);

    let aliases = aliases::AliasContainer {
        labels,
        character_pair: pair,
        outcome,
        count: copied,
    };
    let value = wasm_junction::Val::from(aliases.clone());
    assert_eq!(aliases::AliasContainer::try_from(value).unwrap(), aliases);
}
