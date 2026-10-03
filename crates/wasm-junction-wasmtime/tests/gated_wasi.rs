//! WASI interception coverage for the native engine.

#![cfg(feature = "wasi")]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use wasm_junction::{
    App, Call, CallError, CallErrorKind, Component, Event, InvocationId, Middleware, Next,
    Resource, Val, Vals,
};
use wasm_junction_wasmtime::{GATED_WASI_INTERFACES, WASI_INTERFACES};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const EXPORT: &str = "test:wasi/environment@0.1.0";

struct RecordGates(Arc<Mutex<BTreeSet<(String, String)>>>);

impl Middleware for RecordGates {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        tokio::task::yield_now().await;
        if call.interface.starts_with("wasi:") {
            validate_io_call(&call);
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

fn validate_io_call(call: &Call) {
    let function = call.function.as_ref();
    let expected = match function {
        "[method]input-stream.read"
        | "[method]input-stream.blocking-read"
        | "[method]input-stream.skip"
        | "[method]input-stream.blocking-skip" => &["input-stream", "u64"][..],
        "[method]input-stream.subscribe" => &["input-stream"][..],
        "[method]output-stream.write" | "[method]output-stream.blocking-write-and-flush" => {
            &["output-stream", "bytes"][..]
        }
        "[method]output-stream.check-write"
        | "[method]output-stream.flush"
        | "[method]output-stream.blocking-flush"
        | "[method]output-stream.subscribe" => &["output-stream"][..],
        "[method]output-stream.write-zeroes"
        | "[method]output-stream.blocking-write-zeroes-and-flush" => &["output-stream", "u64"][..],
        "[method]output-stream.splice" | "[method]output-stream.blocking-splice" => {
            &["output-stream", "input-stream", "u64"][..]
        }
        "[method]error.to-debug-string" => &["error"][..],
        "[drop]input-stream" => &["own input-stream"][..],
        "[drop]output-stream" => &["own output-stream"][..],
        "[drop]error" => &["own error"][..],
        "get-random-bytes" | "get-insecure-random-bytes" => &["u64"][..],
        "exit" => &["result"][..],
        "exit-with-code" => &["u8"][..],
        _ => return,
    };
    assert_eq!(
        call.args.len(),
        expected.len(),
        "wrong arguments for {function}"
    );
    for (value, expected) in call.args.iter().zip(expected) {
        match (value, *expected) {
            (Val::U64(_), "u64")
            | (Val::U8(_), "u8")
            | (Val::Bytes(_), "bytes")
            | (Val::Result(Ok(None) | Err(None)), "result") => {}
            (Val::Resource(resource), expected) => {
                let (ownership, name) = expected.strip_prefix("own ").map_or(
                    (wasm_junction::ResourceOwnership::Borrow, expected),
                    |name| (wasm_junction::ResourceOwnership::Own, name),
                );
                assert_eq!(resource.name(), name, "wrong resource for {function}");
                assert_eq!(
                    resource.ownership(),
                    ownership,
                    "wrong ownership for {function}"
                );
                assert_eq!(resource.invocation_id(), Some(call.invocation_id()));
            }
            _ => panic!("wrong value for {function}: {value:?}"),
        }
    }
}

struct RewriteRandom;

impl Middleware for RewriteRandom {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let rewrites = call.interface.as_ref() == "wasi:random/random@0.2.12"
            && call.function.as_ref() == "get-random-bytes";
        let mut values = next.run(call).await?;
        if rewrites {
            values = vec![Val::Bytes(vec![3, 1, 4, 1])];
        }
        Ok(values)
    }
}

struct RefuseRandom;

impl Middleware for RefuseRandom {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "get-random-bytes" {
            Err(CallError::refused("random denied by policy"))
        } else {
            next.run(call).await
        }
    }
}

struct RefuseExit(Arc<Mutex<Option<(String, Vals)>>>);

impl Middleware for RefuseExit {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == "wasi:cli/exit@0.2.12" {
            *self.0.lock().unwrap() = Some((call.function.to_string(), call.args.clone()));
            Err(CallError::refused("exit denied by policy"))
        } else {
            next.run(call).await
        }
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

struct RewriteOversizedWrite;

impl Middleware for RewriteOversizedWrite {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "[method]output-stream.blocking-write-and-flush"
            && matches!(call.args.get(1), Some(Val::Bytes(bytes)) if bytes.len() == 4097)
        {
            call.args[1] = Val::Bytes(b"rewritten".to_vec());
        }
        next.run(call).await
    }
}

struct RecordWriteBytes(Arc<Mutex<Vec<Vec<u8>>>>);

impl Middleware for RecordWriteBytes {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "[method]output-stream.blocking-write-and-flush" {
            let Some(Val::Bytes(bytes)) = call.args.get(1) else {
                panic!("blocking write did not receive bytes");
            };
            self.0.lock().unwrap().push(bytes.clone());
        }
        next.run(call).await
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

struct UnscopeInputStream;

impl Middleware for UnscopeInputStream {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "[method]input-stream.read" {
            let Val::Resource(current) = &call.args[0] else {
                panic!("read did not receive a stream");
            };
            call.args[0] = Val::Resource(Resource::borrowed(
                current.interface(),
                current.name(),
                current.id(),
            ));
        }
        next.run(call).await
    }
}

enum RewriteError {
    Mistyped,
    Unscoped,
}

impl Middleware for RewriteError {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "[method]output-stream.write" {
            return Err(CallError::refused("create an error resource"));
        }
        if call.function.as_ref() == "[method]error.to-debug-string" {
            let Val::Resource(current) = &call.args[0] else {
                panic!("to-debug-string did not receive an error");
            };
            call.args[0] = Val::Resource(match self {
                Self::Mistyped => Resource::__borrowed_for_invocation(
                    "wasi:io/streams@0.2.12",
                    "input-stream",
                    current.id(),
                    call.invocation_id(),
                ),
                Self::Unscoped => {
                    Resource::borrowed(current.interface(), current.name(), current.id())
                }
            });
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChannelObservation {
    open: bool,
    invocation: InvocationId,
    stream: u64,
    direction: wasm_junction::ChannelDirection,
}

struct RecordChannels {
    calls: Arc<Mutex<Vec<InvocationId>>>,
    events: Arc<Mutex<Vec<ChannelObservation>>>,
}

impl Middleware for RecordChannels {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == "wasi:io/streams@0.2.12" {
            self.calls.lock().unwrap().push(call.invocation_id());
        }
        next.run(call).await
    }

    fn event(&self, event: &Event) {
        let observation = match event {
            Event::ChannelOpen {
                invocation,
                stream,
                direction,
            } => Some(ChannelObservation {
                open: true,
                invocation: *invocation,
                stream: *stream,
                direction: *direction,
            }),
            Event::ChannelClose {
                invocation,
                stream,
                direction,
            } => Some(ChannelObservation {
                open: false,
                invocation: *invocation,
                stream: *stream,
                direction: *direction,
            }),
            _ => None,
        };
        if let Some(observation) = observation {
            self.events.lock().unwrap().push(observation);
        }
    }
}
#[test]
fn gated_wasi_set_changes_only_deliberately() {
    assert_eq!(
        GATED_WASI_INTERFACES,
        [
            "wasi:cli/environment@0.2.12",
            "wasi:cli/exit@0.2.12",
            "wasi:cli/stderr@0.2.12",
            "wasi:cli/stdin@0.2.12",
            "wasi:cli/stdout@0.2.12",
            "wasi:clocks/monotonic-clock@0.2.12",
            "wasi:clocks/wall-clock@0.2.12",
            "wasi:io/error@0.2.12",
            "wasi:io/poll@0.2.12",
            "wasi:io/streams@0.2.12",
            "wasi:random/insecure-seed@0.2.12",
            "wasi:random/insecure@0.2.12",
            "wasi:random/random@0.2.12",
        ]
    );
}

#[test]
fn random_bytes_can_be_rewritten() {
    let (app, runtime) = checked_app(RewriteRandom);
    let values = runtime
        .block_on(app.call("checked", EXPORT, "random-bytes", Vec::new()))
        .unwrap();
    assert_eq!(values, [Val::Bytes(vec![3, 1, 4, 1])]);
}

#[test]
fn random_refusal_traps_with_its_kind() {
    let (app, runtime) = checked_app(RefuseRandom);
    let error = runtime
        .block_on(app.call("checked", EXPORT, "random-bytes", Vec::new()))
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "random denied by policy");
}

#[test]
fn refused_exit_is_observed_and_ends_the_invocation() {
    let seen = Arc::new(Mutex::new(None));
    let (app, runtime) = checked_app(RefuseExit(seen.clone()));
    let error = runtime
        .block_on(app.call("checked", EXPORT, "exit-code", Vec::new()))
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "exit denied by policy");
    assert_eq!(
        *seen.lock().unwrap(),
        Some(("exit-with-code".to_owned(), vec![Val::U8(7)]))
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
fn rewritten_write_bytes_reach_wasmtime_wasi() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(RewriteOversizedWrite)
        .middleware(RecordWriteBytes(seen.clone()))
        .build()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("rewrite")))
        .unwrap();
    let values = runtime
        .block_on(app.call("rewrite", EXPORT, "rewritten-write", Vec::new()))
        .unwrap();
    assert_eq!(values, [Val::Bool(true)]);
    assert_eq!(*seen.lock().unwrap(), [b"rewritten".to_vec()]);
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
fn unscoped_input_stream_is_refused() {
    let (app, runtime) = checked_app(UnscopeInputStream);
    let values = runtime
        .block_on(app.call("checked", EXPORT, "input-read", Vec::new()))
        .unwrap();
    let [Val::String(message)] = values.as_slice() else {
        panic!("unscoped input stream returned the wrong shape");
    };
    assert!(message.contains("does not belong to this invocation"));
}

#[test]
fn mistyped_error_handle_is_refused() {
    let (app, runtime) = checked_app(RewriteError::Mistyped);
    let error = runtime
        .block_on(app.call("checked", EXPORT, "refused-write", Vec::new()))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not match the resource type")
    );
}

#[test]
fn unscoped_error_handle_is_refused() {
    let (app, runtime) = checked_app(RewriteError::Unscoped);
    let error = runtime
        .block_on(app.call("checked", EXPORT, "refused-write", Vec::new()))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not belong to this invocation")
    );
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
fn stdout_getter_opens_one_channel_and_drop_closes_it() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(RecordChannels {
            calls: calls.clone(),
            events: events.clone(),
        })
        .build()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("channels")))
        .unwrap();
    runtime
        .block_on(app.call("channels", EXPORT, "stdout-channel", Vec::new()))
        .unwrap();

    let calls = calls.lock().unwrap();
    let events = events.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(events.len(), 2);
    assert!(events[0].open);
    assert!(!events[1].open);
    assert_eq!(events[0].invocation, calls[0]);
    assert_eq!(events[0].stream, events[1].stream);
    assert_eq!(events[0].direction, events[1].direction);
    assert_eq!(events[0].invocation, events[1].invocation);
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
    runtime
        .block_on(app.call("coverage", EXPORT, "exit-success", Vec::new()))
        .unwrap_err();
    let exit = runtime
        .block_on(app.call("coverage", EXPORT, "exit-code", Vec::new()))
        .unwrap_err();
    assert_eq!(exit.kind(), CallErrorKind::Trap);
    assert!(exit.to_string().contains('7'));

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
