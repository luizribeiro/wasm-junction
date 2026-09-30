//! Shared handwritten bindings for integration tests.

#![allow(
    dead_code,
    reason = "each integration test uses a different subset of the shared bindings"
)]

use wasm_junction::{Call, Caller, TypeError, TypedCall, Val, Vals};
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

/// The interface implemented by the handwritten notes fixtures.
pub const NOTES: &str = "example:journal/notes@0.1.0";

/// Builds a real component from inline WIT and a matching dummy core module.
pub fn component_bytes(wit: &str, world_name: &str) -> Vec<u8> {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("fixture.wit", wit).unwrap();
    let world = resolve.select_world(&[package], Some(world_name)).unwrap();
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

/// Typed arguments for the notes `read` function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// The note name to read.
    pub name: String,
}

impl TypedCall for Read {
    type Output = String;

    const INTERFACE: &'static str = NOTES;
    const FUNCTION: &'static str = "read";

    fn from_vals(values: &[Val]) -> Result<Self, TypeError> {
        let [Val::String(name)] = values else {
            return Err(TypeError::new("notes.read expects one string"));
        };
        Ok(Self { name: name.clone() })
    }

    fn into_vals(self) -> Vals {
        vec![self.name.into()]
    }

    fn output(value: Self::Output) -> Vals {
        vec![value.into()]
    }

    fn decode_output(values: &[Val]) -> Result<Self::Output, TypeError> {
        let [Val::String(value)] = values else {
            return Err(TypeError::new("notes.read returns one string"));
        };
        Ok(value.clone())
    }
}

/// Typed arguments for the notes `delete` function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delete {
    /// The note name to delete.
    pub name: String,
}

impl TypedCall for Delete {
    type Output = ();

    const INTERFACE: &'static str = NOTES;
    const FUNCTION: &'static str = "delete";

    fn from_vals(values: &[Val]) -> Result<Self, TypeError> {
        let [Val::String(name)] = values else {
            return Err(TypeError::new("notes.delete expects one string"));
        };
        Ok(Self { name: name.clone() })
    }

    fn into_vals(self) -> Vals {
        vec![self.name.into()]
    }

    fn output(_value: Self::Output) -> Vals {
        Vec::new()
    }

    fn decode_output(values: &[Val]) -> Result<Self::Output, TypeError> {
        if values.is_empty() {
            Ok(())
        } else {
            Err(TypeError::new("notes.delete returns no values"))
        }
    }
}

/// Builds a notes `read` invocation from the summarizer component.
pub fn read_call(name: &str) -> Call {
    Call::new(
        Caller::Component(String::from("summarizer")),
        "notebook",
        NOTES,
        "read",
        vec![name.into()],
    )
}
