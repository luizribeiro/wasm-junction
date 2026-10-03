//! WASI interception coverage for the native engine.

#![cfg(feature = "wasi")]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use wasm_junction::{
    App, Call, CallError, Component, Event, InvocationId, Middleware, Next, Resource, Val, Vals,
};
use wasm_junction_wasmtime::{GATED_WASI_INTERFACES, WASI_INTERFACES};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const EXPORT: &str = "test:wasi/environment@0.1.0";

struct RecordGates(Arc<Mutex<BTreeSet<(String, String)>>>);

impl Middleware for RecordGates {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.starts_with("wasi:") {
            self.0
                .lock()
                .unwrap()
                .insert((call.interface.to_string(), call.function.to_string()));
        }
        if call.function.as_ref() == "[method]output-stream.write"
            && call.args.get(1) == Some(&Val::Bytes(vec![0xfa]))
        {
            return Err(CallError::refused("fixture write denied"));
        }
        next.run(call).await
    }
}

struct RefuseWrite;

impl Middleware for RefuseWrite {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "[method]output-stream.write" {
            Err(CallError::refused("write denied by policy"))
        } else {
            next.run(call).await
        }
    }
}

enum RewriteOutputStream {
    Foreign(Mutex<Option<Resource>>),
    Mistyped,
    Unscoped,
}

impl Middleware for RewriteOutputStream {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "[method]output-stream.write" {
            let invocation = call.invocation_id();
            let Val::Resource(current) = &call.args[0] else {
                panic!("write did not receive a stream");
            };
            let replacement = match self {
                Self::Foreign(saved) => {
                    let mut saved = saved.lock().unwrap();
                    if let Some(foreign) = saved.as_ref() {
                        Some(foreign.clone())
                    } else {
                        *saved = Some(current.clone());
                        None
                    }
                }
                Self::Mistyped => Some(Resource::__borrowed_for_invocation(
                    "wasi:io/streams@0.2.12",
                    "input-stream",
                    current.id(),
                    invocation,
                )),
                Self::Unscoped => Some(Resource::borrowed(
                    current.interface(),
                    current.name(),
                    current.id(),
                )),
            };
            if let Some(replacement) = replacement {
                call.args[0] = Val::Resource(replacement);
            }
        }
        next.run(call).await
    }
}

struct RecordResourceScope(Arc<Mutex<Option<(InvocationId, InvocationId)>>>);

impl Middleware for RecordResourceScope {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let invocation = call.invocation_id();
        let records_subscription = call.function.as_ref() == "subscribe-duration";
        let values = next.run(call).await?;
        if records_subscription {
            let Val::Resource(resource) = &values[0] else {
                panic!("subscription did not return a resource");
            };
            *self.0.lock().unwrap() = Some((invocation, resource.invocation_id().unwrap()));
        }
        Ok(values)
    }
}

enum RewritePollable {
    Foreign(Mutex<Option<Resource>>),
    Mistyped,
    Unscoped,
}

impl Middleware for RewritePollable {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "[method]pollable.ready" {
            let invocation = call.invocation_id();
            let Val::Resource(current) = &call.args[0] else {
                panic!("ready did not receive a resource");
            };
            let replacement = match self {
                Self::Foreign(saved) => {
                    let mut saved = saved.lock().unwrap();
                    if let Some(foreign) = saved.as_ref() {
                        Some(foreign.clone())
                    } else {
                        *saved = Some(current.clone());
                        None
                    }
                }
                Self::Mistyped => Some(Resource::__borrowed_for_invocation(
                    "test:wrong/handle@1.0.0",
                    "wrong",
                    current.id(),
                    invocation,
                )),
                Self::Unscoped => Some(Resource::borrowed(
                    current.interface(),
                    current.name(),
                    current.id(),
                )),
            };
            if let Some(replacement) = replacement {
                call.args[0] = Val::Resource(replacement);
            }
        }
        next.run(call).await
    }
}

fn checked_app(middleware: impl Middleware + 'static) -> (App, tokio::runtime::Runtime) {
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(middleware)
        .build()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("checked")))
        .unwrap();
    (app, runtime)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DropObservation {
    invocation: InvocationId,
    interface: String,
    resource: String,
}

struct RecordPollableDrop(Arc<Mutex<Vec<DropObservation>>>);

impl Middleware for RecordPollableDrop {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "[drop]pollable" {
            self.0.lock().unwrap().push(DropObservation {
                invocation: call.invocation_id(),
                interface: call.interface.to_string(),
                resource: "pollable".to_owned(),
            });
        }
        next.run(call).await
    }

    fn event(&self, event: &Event) {
        if let Event::ResourceDrop {
            invocation,
            interface,
            resource,
            ..
        } = event
        {
            self.0.lock().unwrap().push(DropObservation {
                invocation: *invocation,
                interface: interface.to_string(),
                resource: resource.to_string(),
            });
        }
    }
}
#[test]
fn gated_wasi_set_changes_only_deliberately() {
    assert_eq!(
        GATED_WASI_INTERFACES,
        [
            "wasi:cli/environment@0.2.12",
            "wasi:clocks/monotonic-clock@0.2.12",
            "wasi:clocks/wall-clock@0.2.12",
            "wasi:io/error@0.2.12",
            "wasi:io/poll@0.2.12",
            "wasi:io/streams@0.2.12",
        ]
    );
}

#[test]
fn advertised_wasi_set_excludes_ungated_filesystem_and_sockets() {
    assert!(
        WASI_INTERFACES
            .iter()
            .all(|interface| !interface.starts_with("wasi:filesystem/")
                && !interface.starts_with("wasi:sockets/"))
    );
}

#[test]
fn wasi_resources_carry_their_invocation() {
    let seen = Arc::new(Mutex::new(None));
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(RecordResourceScope(seen.clone()))
        .build()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("scope")))
        .unwrap();
    runtime
        .block_on(app.call("scope", EXPORT, "start-timer", Vec::new()))
        .unwrap();

    let (call, resource) = seen.lock().unwrap().unwrap();
    assert_eq!(resource, call);
}

#[test]
fn foreign_pollable_is_refused() {
    let (app, runtime) = checked_app(RewritePollable::Foreign(Mutex::new(None)));
    runtime
        .block_on(app.call("checked", EXPORT, "coverage", Vec::new()))
        .unwrap();
    let error = runtime
        .block_on(app.call("checked", EXPORT, "coverage", Vec::new()))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not belong to this invocation")
    );
}

#[test]
fn mistyped_pollable_is_refused() {
    let (app, runtime) = checked_app(RewritePollable::Mistyped);
    let error = runtime
        .block_on(app.call("checked", EXPORT, "coverage", Vec::new()))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not match the resource type")
    );
}

#[test]
fn unscoped_pollable_is_refused() {
    let (app, runtime) = checked_app(RewritePollable::Unscoped);
    let error = runtime
        .block_on(app.call("checked", EXPORT, "coverage", Vec::new()))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not belong to this invocation")
    );
}

#[test]
fn refused_write_is_a_guest_stream_error() {
    let (app, runtime) = checked_app(RefuseWrite);
    let values = runtime
        .block_on(app.call("checked", EXPORT, "refused-write", Vec::new()))
        .unwrap();
    let [Val::String(message)] = values.as_slice() else {
        panic!("refused write returned the wrong shape");
    };
    assert!(message.contains("write denied by policy"));
}

#[test]
fn foreign_output_stream_is_refused() {
    let (app, runtime) = checked_app(RewriteOutputStream::Foreign(Mutex::new(None)));
    runtime
        .block_on(app.call("checked", EXPORT, "refused-write", Vec::new()))
        .unwrap();
    let values = runtime
        .block_on(app.call("checked", EXPORT, "refused-write", Vec::new()))
        .unwrap();
    let [Val::String(message)] = values.as_slice() else {
        panic!("foreign stream returned the wrong shape");
    };
    assert!(message.contains("does not belong to this invocation"));
}

#[test]
fn mistyped_output_stream_is_refused() {
    let (app, runtime) = checked_app(RewriteOutputStream::Mistyped);
    let values = runtime
        .block_on(app.call("checked", EXPORT, "refused-write", Vec::new()))
        .unwrap();
    let [Val::String(message)] = values.as_slice() else {
        panic!("mistyped stream returned the wrong shape");
    };
    assert!(message.contains("does not match the resource type"));
}

#[test]
fn unscoped_output_stream_is_refused() {
    let (app, runtime) = checked_app(RewriteOutputStream::Unscoped);
    let values = runtime
        .block_on(app.call("checked", EXPORT, "refused-write", Vec::new()))
        .unwrap();
    let [Val::String(message)] = values.as_slice() else {
        panic!("unscoped stream returned the wrong shape");
    };
    assert!(message.contains("does not belong to this invocation"));
}

#[test]
fn pollable_drop_is_a_call_and_an_event() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(RecordPollableDrop(seen.clone()))
        .build()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("drop")))
        .unwrap();
    runtime
        .block_on(app.call("drop", EXPORT, "start-timer", Vec::new()))
        .unwrap();

    let drops = seen.lock().unwrap();
    assert_eq!(drops.len(), 2);
    assert_eq!(drops[0], drops[1]);
    assert_eq!(drops[0].interface, "wasi:io/poll@0.2.12");
    assert_eq!(drops[0].resource, "pollable");
}

#[test]
fn every_function_in_each_gated_wit_interface_has_a_gate() {
    let seen = Arc::new(Mutex::new(BTreeSet::new()));
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(RecordGates(seen.clone()))
        .build()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("coverage")))
        .unwrap();
    runtime
        .block_on(app.call("coverage", EXPORT, "coverage", Vec::new()))
        .unwrap();

    assert_eq!(*seen.lock().unwrap(), wit_functions());
}

fn wit_functions() -> BTreeSet<(String, String)> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let resolved: Vec<_> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|package| package["name"] == "wasmtime-wasi")
        .collect();
    assert_eq!(resolved.len(), 1, "expected one resolved wasmtime-wasi");
    let manifest = resolved[0]["manifest_path"].as_str().unwrap();
    let wit = Path::new(manifest).parent().unwrap().join("src/p2/wit");
    let mut resolve = wit_parser::Resolve::default();
    resolve.push_dir(wit).unwrap();

    resolve
        .packages
        .iter()
        .flat_map(|(_, package)| {
            package.interfaces.iter().filter_map(|(name, interface)| {
                let name = package.name.interface_id(name);
                GATED_WASI_INTERFACES
                    .contains(&name.as_str())
                    .then_some((name, &resolve.interfaces[*interface]))
            })
        })
        .flat_map(|(interface_name, interface)| {
            interface
                .functions
                .keys()
                .map(move |function| (interface_name.clone(), function.clone()))
        })
        .chain([
            ("wasi:io/error@0.2.12".to_owned(), "[drop]error".to_owned()),
            (
                "wasi:io/poll@0.2.12".to_owned(),
                "[drop]pollable".to_owned(),
            ),
            (
                "wasi:io/streams@0.2.12".to_owned(),
                "[drop]input-stream".to_owned(),
            ),
            (
                "wasi:io/streams@0.2.12".to_owned(),
                "[drop]output-stream".to_owned(),
            ),
        ])
        .collect()
}
