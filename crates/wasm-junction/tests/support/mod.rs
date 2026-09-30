//! Shared handwritten bindings for integration tests.

#![allow(
    dead_code,
    reason = "each integration test uses a different subset of the shared bindings"
)]

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use wasm_junction::{
    BoxFuture, Call, CallContext, Caller, CompiledComponent, Engine, ImportDispatcher,
    InvocationContext, Provider, Trap, TypeError, TypedCall, Val, Vals,
};
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

/// The interface implemented by the handwritten notes fixtures.
pub const NOTES: &str = "example:journal/notes@0.1.0";

/// Builds a real component from inline WIT and a matching dummy core module.
pub fn component_bytes(wit: &str, world_name: &str) -> Vec<u8> {
    component_bytes_from(&[("fixture.wit", wit)], world_name)
}

/// Builds a component from multiple WIT packages.
pub fn component_bytes_from(packages: &[(&str, &str)], world_name: &str) -> Vec<u8> {
    let mut resolve = Resolve::default();
    let packages = packages
        .iter()
        .map(|(name, wit)| resolve.push_str(name, wit).unwrap())
        .collect::<Vec<_>>();
    let world = resolve.select_world(&packages, Some(world_name)).unwrap();
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

/// Drives a fixture future that does not depend on an executor.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

/// An engine that compiles to an export which is never called.
pub struct FakeEngine;

impl Engine for FakeEngine {
    fn compile(
        &self,
        _bytes: Arc<[u8]>,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, Trap>> {
        Box::pin(async { Ok(Arc::new(UnusedComponent) as Arc<dyn CompiledComponent>) })
    }
}

struct UnusedComponent;

impl CompiledComponent for UnusedComponent {
    fn call(
        &self,
        imports: Arc<dyn ImportDispatcher>,
        context: InvocationContext,
        component: Arc<str>,
        interface: Arc<str>,
        function: Arc<str>,
        args: Vals,
    ) -> BoxFuture<'_, Result<Vals, Trap>> {
        Box::pin(async move {
            if interface.as_ref() != "example:journal/summaries@0.1.0" {
                return Err(Trap::new("engine received an unresolved export"));
            }
            if function.as_ref() != "summarize" {
                return Err(Trap::new("engine received the wrong function"));
            }
            imports
                .call(
                    context,
                    component,
                    Arc::from(NOTES),
                    Arc::from("read"),
                    args,
                )
                .await
        })
    }
}

/// A provider used only to satisfy a component import.
pub struct UnusedProvider;

impl Provider for UnusedProvider {
    fn call<'a>(&'a self, _cx: &'a CallContext, _call: Call) -> BoxFuture<'a, Result<Vals, Trap>> {
        Box::pin(async { Err(Trap::new("unused provider")) })
    }
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
        Caller::Component(Arc::from("summarizer")),
        "notebook",
        NOTES,
        "read",
        vec![name.into()],
    )
}
