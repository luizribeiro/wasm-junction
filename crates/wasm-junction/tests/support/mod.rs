//! Shared handwritten bindings for integration tests.

#![allow(
    dead_code,
    reason = "each integration test uses a different subset of the shared bindings"
)]

use wasm_junction::{Call, Caller, TypeError, TypedCall, Val, Vals};

/// The interface implemented by the handwritten notes fixtures.
pub const NOTES: &str = "example:journal/notes@0.1.0";

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
