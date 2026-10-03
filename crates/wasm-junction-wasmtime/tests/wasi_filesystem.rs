//! WASI Preview 2 filesystem behavior.

#![cfg(feature = "wasi")]

use std::path::PathBuf;

use wasm_junction::{Access, App, Component, Val, WasiSettings};

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
