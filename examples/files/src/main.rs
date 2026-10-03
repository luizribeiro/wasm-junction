//! A policy-gated guest reading files from temporary preopened directories.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod middleware;

use std::error::Error;
use std::path::PathBuf;

use middleware::{PreopenPolicy, Trace};
use wasm_junction::{Access, App, Component, Val, WasiSettings, wasi};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/files.wasm"));
const FILES: &str = "example:files/files@0.1.0";

struct TemporaryFiles(PathBuf);

impl TemporaryFiles {
    fn create() -> std::io::Result<Self> {
        let root = std::env::temp_dir().join(format!(
            "wasm-junction-files-example-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (directory, contents) in [
            ("allowed", "a readable note"),
            ("blocked", "a private note"),
        ] {
            let path = root.join(directory);
            std::fs::create_dir_all(&path)?;
            std::fs::write(path.join("note.txt"), contents)?;
        }
        Ok(Self(root))
    }

    fn directory(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TemporaryFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let files = TemporaryFiles::create()?;
    let app = App::builder()
        .provide(wasi::provider())
        .middleware(Trace)
        .middleware(PreopenPolicy::deny("/blocked"))
        .build()?;
    app.configure(
        "files",
        WasiSettings::new()
            .preopen(files.directory("allowed"), "/allowed", Access::ReadOnly)
            .preopen(files.directory("blocked"), "/blocked", Access::ReadOnly),
    )?;
    app.load(Component::from_bytes(COMPONENT)?.named("files"))
        .await?;

    println!("allowed: {}", read(&app, "/allowed/note.txt").await?);
    println!("blocked: {}", read(&app, "/blocked/note.txt").await?);
    Ok(())
}

async fn read(app: &App, path: &str) -> Result<String, Box<dyn Error>> {
    let values = app
        .call("files", FILES, "read", vec![Val::from(path)])
        .await?;
    let [Val::String(result)] = values.as_slice() else {
        return Err("files guest returned the wrong shape".into());
    };
    Ok(result.clone())
}
