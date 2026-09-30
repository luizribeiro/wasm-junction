//! Component inspection tests.

use wasm_junction::{Component, ComponentError};
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

fn component_bytes() -> Vec<u8> {
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str(
            "journal.wit",
            r"
                package example:journal@0.1.0;
                interface notes { read: func(name: string) -> string; }
                interface summaries { summarize: func(text: string) -> string; }
                world plugin { import notes; export summaries; }
            ",
        )
        .unwrap();
    let world = resolve.select_world(&[package], Some("plugin")).unwrap();
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

#[test]
fn bytes_expose_versioned_interfaces_without_an_engine() {
    let component = Component::from_bytes(component_bytes())
        .unwrap()
        .named("summarizer");

    assert_eq!(component.name(), Some("summarizer"));
    assert_eq!(component.imports(), ["example:journal/notes@0.1.0"]);
    assert_eq!(component.exports(), ["example:journal/summaries@0.1.0"]);
}

#[test]
fn core_modules_and_malformed_bytes_are_rejected() {
    let module = Component::from_bytes(b"\0asm\x01\0\0\0".as_slice()).unwrap_err();
    assert!(matches!(module, ComponentError::CoreModule));

    let malformed = Component::from_bytes(b"not wasm".as_slice()).unwrap_err();
    assert!(matches!(malformed, ComponentError::Parse(_)));
}
