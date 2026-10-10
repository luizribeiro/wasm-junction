use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Type};
use wasmtime::{Config, Engine};
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

pub(crate) fn item_type(declarations: &str, item: &str) -> Type {
    let wit = format!(
        "package example:test@1.0.0; {declarations} world test {{ export check: func(value: stream<{item}>); }}"
    );
    let (engine, component) = component(&wit);
    let function = component
        .component_type()
        .exports(&engine)
        .find_map(|(name, item)| (name == "check").then_some(item.ty))
        .unwrap();
    let ComponentItem::ComponentFunc(function) = function else {
        panic!("check export is not a function")
    };
    let (_, Type::Stream(stream)) = function.params().next().unwrap() else {
        panic!("check parameter is not a stream")
    };
    stream.ty().unwrap()
}

pub(crate) fn component_with_record_stream_import(
    package: &str,
    interface: &str,
) -> (Engine, Component) {
    component(&format!(
        "package {package}; interface {interface} {{ record entry {{ name: string }} consume: func(value: stream<entry>); }} world test {{ import {interface}; }}"
    ))
}

fn component(wit: &str) -> (Engine, Component) {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("test.wit", wit).unwrap();
    let world = resolve.select_world(&[package], Some("test")).unwrap();
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();
    let bytes = ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();
    let mut config = Config::new();
    config.wasm_component_model_async(true);
    let engine = Engine::new(&config).unwrap();
    let component = Component::new(&engine, bytes).unwrap();
    (engine, component)
}
