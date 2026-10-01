//! Tests for reusing generated WIT packages.

#![forbid(unsafe_code)]

mod shared {
    wasm_junction::bindgen!({ path: "tests/fixtures/dependencies/wit/deps" });
}

mod consumer {
    wasm_junction::bindgen!({
        path: "tests/fixtures/dependencies/wit",
        with: { "test:common": crate::shared },
    });
}

#[test]
fn reused_types_pass_between_binding_modules() {
    let author = shared::types::Author { name: "Ada".into() };
    let note = consumer::notes::Note { author };

    let shared::types::Author { name } = note.author;
    assert_eq!(name, "Ada");
}
