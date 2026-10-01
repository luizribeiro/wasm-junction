//! Shared handwritten bindings for integration tests.

#![allow(
    dead_code,
    reason = "each integration test uses a different subset of the shared bindings"
)]

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use wasm_junction::{
    BoxFuture, Call, CallContext, CallError, Caller, CompiledComponent, Engine, EngineError,
    ImportDispatcher, InvocationContext, Provider, TypeError, TypedCall, Val, Vals, WasiConfig,
};
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

/// The interface implemented by the handwritten notes fixtures.
pub const NOTES: &str = "example:journal/notes@0.1.0";

/// The generated-handle fixture interface.
pub const HANDLE_SUMMARIES: &str = "test:handles/summaries@1.0.0";

/// The reserved interface-name fixture.
pub const RESERVED_HOST: &str = "test:keywords/host";

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
        _wasi: WasiConfig,
    ) -> BoxFuture<'_, Result<Arc<dyn CompiledComponent>, EngineError>> {
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
    ) -> BoxFuture<'_, Result<Vals, CallError>> {
        Box::pin(async move {
            if interface.as_ref() == "example:cycle/first-api@1.0.0" {
                return imports
                    .call(
                        context,
                        component,
                        Arc::from("example:cycle/second-api@1.0.0"),
                        function,
                        args,
                    )
                    .await;
            }
            if interface.as_ref() == "example:cycle/second-api@1.0.0" {
                return imports
                    .call(
                        context,
                        component,
                        Arc::from("example:cycle/first-api@1.0.0"),
                        function,
                        args,
                    )
                    .await;
            }
            if interface.as_ref() == "example:writer/article@1.0.0" {
                return imports
                    .call(
                        context,
                        component,
                        Arc::from("example:translate/translator@0.1.0"),
                        Arc::from("translate"),
                        args,
                    )
                    .await;
            }
            if interface.as_ref() == "example:translate/translator@0.1.7" {
                let [wasm_junction::Val::String(text)] = args.as_slice() else {
                    return Err(CallError::trap("translator expected one string"));
                };
                return Ok(vec![format!("translated: {text}").into()]);
            }
            if interface.as_ref() == RESERVED_HOST {
                return imports
                    .call(context, component, interface, function, args)
                    .await;
            }
            if interface.as_ref() == HANDLE_SUMMARIES {
                return handle_call(&function, args);
            }
            if interface.as_ref() != "example:journal/summaries@0.1.0" {
                return Err(CallError::trap("engine received an unresolved export"));
            }
            let import = match function.as_ref() {
                "summarize" => "read",
                "unknown" => "unknown",
                _ => return Err(CallError::trap("engine received the wrong function")),
            };
            imports
                .call(
                    context,
                    component,
                    Arc::from(NOTES),
                    Arc::from(import),
                    args,
                )
                .await
        })
    }
}

fn handle_call(function: &str, args: Vals) -> Result<Vals, CallError> {
    match function {
        "summarize" => {
            let expected = vec![
                Val::String("today".into()),
                Val::Record(vec![("title".into(), Val::String("project".into()))]),
                Val::Variant {
                    case: "one".into(),
                    value: Some(Box::new(Val::String("open".into()))),
                },
                Val::List(vec![Val::U32(2), Val::U32(4)]),
                Val::U32(10),
            ];
            if args != expected {
                return Err(CallError::trap("handle arguments were encoded incorrectly"));
            }
            Ok(vec![Val::String("summary".into())])
        }
        "accepted" => Ok(vec![Val::Result(Ok(Some(Box::new(Val::String(
            "saved".into(),
        )))))]),
        "rejected" => Ok(vec![Val::Result(Err(Some(Box::new(Val::String(
            "denied".into(),
        )))))]),
        "clone" | "f" | "from-app" | "g" | "with" | "within" => Ok(args),
        "inspect" => {
            let expected = vec![
                Val::Result(Ok(Some(Box::new(Val::String("done".into()))))),
                Val::Tuple(vec![Val::String("pair".into()), Val::U32(7)]),
            ];
            if args != expected {
                return Err(CallError::trap(
                    "borrowed arguments were encoded incorrectly",
                ));
            }
            Ok(Vec::new())
        }
        _ => Err(CallError::trap("engine received the wrong function")),
    }
}

/// A provider used only to satisfy a component import.
pub struct UnusedProvider;

impl Provider for UnusedProvider {
    fn call<'a>(
        &'a self,
        _cx: &'a CallContext,
        _call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async { Err(CallError::trap("unused provider")) })
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
