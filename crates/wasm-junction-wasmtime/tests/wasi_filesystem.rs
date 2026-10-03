//! WASI Preview 2 filesystem behavior.

#![cfg(feature = "wasi")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use wasm_junction::{
    Access, App, Call, CallError, Component, Middleware, Next, Val, Vals, WasiSettings,
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
