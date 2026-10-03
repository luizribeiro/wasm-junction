//! WASI Preview 2 filesystem behavior.

#![cfg(feature = "wasi")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use wasm_junction::{
    Access, App, Call, CallError, ChannelDirection, Component, Event, InvocationId, Middleware,
    Next, Val, Vals, WasiSettings,
};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const EXPORT: &str = "test:wasi/environment@0.1.0";

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("wasm-junction-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("note.txt"), "a filesystem note").unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn app() -> (App, tokio::runtime::Runtime) {
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("files")))
        .unwrap();
    (app, runtime)
}

struct ObserveFilesystem {
    refuse_open: bool,
    seen: Arc<Mutex<Vec<(String, Vals)>>>,
}

impl Middleware for ObserveFilesystem {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == "wasi:filesystem/types@0.2.12"
            && matches!(
                call.function.as_ref(),
                "[method]descriptor.open-at"
                    | "[method]descriptor.stat"
                    | "[method]descriptor.link-at"
            )
        {
            self.seen
                .lock()
                .unwrap()
                .push((call.function.to_string(), call.args.clone()));
        }
        if self.refuse_open && call.function.as_ref() == "[method]descriptor.open-at" {
            Err(CallError::refused("filesystem policy denied open"))
        } else {
            next.run(call).await
        }
    }
}

fn observed_app(
    refuse_open: bool,
    seen: Arc<Mutex<Vec<(String, Vals)>>>,
) -> (App, tokio::runtime::Runtime) {
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(ObserveFilesystem { refuse_open, seen })
        .build()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("files")))
        .unwrap();
    (app, runtime)
}

#[test]
fn preopens_default_empty_and_live_configuration_is_readable() {
    let directory = TestDirectory::new("filesystem-live");
    let (app, runtime) = app();
    let empty = runtime
        .block_on(app.call("files", EXPORT, "directories", Vec::new()))
        .unwrap();
    assert_eq!(empty, [Val::List(Vec::new())]);

    app.configure(
        "files",
        WasiSettings::new().preopen(&directory.0, "/data", Access::ReadOnly),
    )
    .unwrap();
    let directories = runtime
        .block_on(app.call("files", EXPORT, "directories", Vec::new()))
        .unwrap();
    assert_eq!(directories, [Val::List(vec![Val::from("/data")])]);
    let contents = runtime
        .block_on(app.call(
            "files",
            EXPORT,
            "read-file",
            vec![Val::from("/data/note.txt")],
        ))
        .unwrap();
    assert_eq!(
        contents,
        [Val::Result(Ok(Some(Box::new(Val::from(
            "a filesystem note"
        )))))]
    );
}

#[test]
fn read_only_preopen_refuses_std_writes() {
    let directory = TestDirectory::new("filesystem-read-only");
    let (app, runtime) = app();
    app.configure(
        "files",
        WasiSettings::new().preopen(&directory.0, "/data", Access::ReadOnly),
    )
    .unwrap();
    let result = runtime
        .block_on(app.call(
            "files",
            EXPORT,
            "write-file",
            vec![Val::from("/data/new.txt"), Val::from("blocked")],
        ))
        .unwrap();
    assert_eq!(
        result,
        [Val::Result(Err(Some(Box::new(Val::from(
            "PermissionDenied"
        )))))]
    );
    assert!(!directory.0.join("new.txt").exists());
}

#[test]
fn middleware_refusal_becomes_permission_denied() {
    let directory = TestDirectory::new("filesystem-refusal");
    let (app, runtime) = observed_app(true, Arc::new(Mutex::new(Vec::new())));
    app.configure(
        "files",
        WasiSettings::new().preopen(&directory.0, "/data", Access::ReadOnly),
    )
    .unwrap();
    let result = runtime
        .block_on(app.call(
            "files",
            EXPORT,
            "read-file",
            vec![Val::from("/data/note.txt")],
        ))
        .unwrap();
    assert_eq!(
        result,
        [Val::Result(Err(Some(Box::new(Val::from(
            "PermissionDenied"
        )))))]
    );
}

#[test]
fn descriptor_calls_carry_preopen_roots_in_descriptor_order() {
    let directory = TestDirectory::new("filesystem-context");
    let peer = TestDirectory::new("filesystem-context-peer");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (app, runtime) = observed_app(false, seen.clone());
    app.configure(
        "files",
        WasiSettings::new()
            .preopen(&directory.0, "/data", Access::ReadWrite)
            .preopen(&peer.0, "/peer", Access::ReadWrite),
    )
    .unwrap();
    runtime
        .block_on(app.call(
            "files",
            EXPORT,
            "read-file",
            vec![Val::from("/data/note.txt")],
        ))
        .unwrap();
    runtime
        .block_on(app.call("files", EXPORT, "stat-preopen", Vec::new()))
        .unwrap();
    runtime
        .block_on(app.call("files", EXPORT, "link-preopens", Vec::new()))
        .unwrap();

    let seen = seen.lock().unwrap();
    for function in ["[method]descriptor.open-at", "[method]descriptor.stat"] {
        let (_, args) = seen.iter().find(|(name, _)| name == function).unwrap();
        assert_eq!(args.last(), Some(&Val::from("/data")));
    }
    let (_, args) = seen
        .iter()
        .find(|(name, _)| name == "[method]descriptor.link-at")
        .unwrap();
    assert_eq!(
        args[args.len() - 2..],
        [Val::from("/data"), Val::from("/peer")]
    );
}

type ChannelEvents = Arc<Mutex<Vec<(bool, InvocationId, u64, ChannelDirection)>>>;

struct RecordChannels {
    calls: Arc<Mutex<Vec<(InvocationId, ChannelDirection)>>>,
    events: ChannelEvents,
}

impl Middleware for RecordChannels {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let direction = match call.function.as_ref() {
            "[method]descriptor.read-via-stream" => Some(ChannelDirection::HostToGuest),
            "[method]descriptor.write-via-stream" | "[method]descriptor.append-via-stream" => {
                Some(ChannelDirection::GuestToHost)
            }
            _ => None,
        };
        if let Some(direction) = direction {
            self.calls
                .lock()
                .unwrap()
                .push((call.invocation_id(), direction));
        }
        next.run(call).await
    }

    fn event(&self, event: &Event) {
        let observed = match event {
            Event::ChannelOpen {
                invocation,
                stream,
                direction,
            } => Some((true, *invocation, *stream, *direction)),
            Event::ChannelClose {
                invocation,
                stream,
                direction,
            } => Some((false, *invocation, *stream, *direction)),
            _ => None,
        };
        if let Some(observed) = observed {
            self.events.lock().unwrap().push(observed);
        }
    }
}

#[test]
fn filesystem_streams_open_and_close_invocation_channels() {
    let directory = TestDirectory::new("filesystem-channels");
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
    app.configure(
        "files",
        WasiSettings::new().preopen(&directory.0, "/data", Access::ReadWrite),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("files")))
        .unwrap();
    runtime
        .block_on(app.call("files", EXPORT, "filesystem-channels", Vec::new()))
        .unwrap();

    let calls = calls.lock().unwrap();
    let events = events.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(events.len(), 6);
    for (invocation, direction) in calls.iter() {
        let matching = events
            .iter()
            .filter(|event| event.1 == *invocation && event.3 == *direction)
            .collect::<Vec<_>>();
        assert!(matching.iter().any(|event| event.0));
        assert!(matching.iter().any(|event| !event.0));
    }
    for opened in events.iter().filter(|event| event.0) {
        assert!(events.iter().any(|closed| {
            !closed.0 && closed.1 == opened.1 && closed.2 == opened.2 && closed.3 == opened.3
        }));
    }
}

#[derive(Clone, Copy)]
enum InvalidHandle {
    Foreign,
    Mistyped,
    Unscoped,
}

struct RewriteHandle {
    target: &'static str,
    replacement: InvalidHandle,
    saved: Mutex<Option<wasm_junction::Resource>>,
}

impl Middleware for RewriteHandle {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() != self.target {
            return next.run(call).await;
        }
        let Val::Resource(current) = &call.args[0] else {
            panic!("filesystem handle call did not receive a resource")
        };
        let replacement = match self.replacement {
            InvalidHandle::Foreign => {
                let foreign = {
                    let mut saved = self.saved.lock().unwrap();
                    if let Some(resource) = saved.as_ref() {
                        Some(resource.clone())
                    } else {
                        *saved = Some(current.clone());
                        None
                    }
                };
                let Some(resource) = foreign else {
                    return next.run(call).await;
                };
                resource
            }
            InvalidHandle::Mistyped => wasm_junction::Resource::__borrowed_for_invocation(
                "wasi:io/poll@0.2.12",
                "pollable",
                current.id(),
                call.invocation_id(),
            ),
            InvalidHandle::Unscoped => {
                wasm_junction::Resource::borrowed(current.interface(), current.name(), current.id())
            }
        };
        call.args[0] = Val::Resource(replacement);
        next.run(call).await
    }
}

fn assert_invalid_handle(export: &str, target: &'static str, replacement: InvalidHandle) {
    let directory = TestDirectory::new(export);
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(RewriteHandle {
            target,
            replacement,
            saved: Mutex::new(None),
        })
        .build()
        .unwrap();
    app.configure(
        "files",
        WasiSettings::new().preopen(&directory.0, "/data", Access::ReadOnly),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("files")))
        .unwrap();
    if matches!(replacement, InvalidHandle::Foreign) {
        runtime
            .block_on(app.call("files", EXPORT, export, Vec::new()))
            .unwrap();
    }
    let result = runtime
        .block_on(app.call("files", EXPORT, export, Vec::new()))
        .unwrap();
    assert_eq!(result, [Val::Bool(true)]);
}

#[test]
fn invalid_filesystem_handles_are_refused_as_access_errors() {
    for (export, target) in [
        ("descriptor-handle", "[method]descriptor.stat"),
        (
            "directory-stream-handle",
            "[method]directory-entry-stream.read-directory-entry",
        ),
    ] {
        for replacement in [
            InvalidHandle::Foreign,
            InvalidHandle::Mistyped,
            InvalidHandle::Unscoped,
        ] {
            assert_invalid_handle(export, target, replacement);
        }
    }
}
