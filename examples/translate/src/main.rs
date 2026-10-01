//! Routes one component's import between two translator components.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
mod trace;

use std::error::Error;

use bindings::{translator, writer};
use trace::Trace;
use wasm_junction::{App, Component, LoadError};

const DEEPL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/deepl.wasm"));
const GOOGLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/google.wasm"));
const WRITER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/writer.wasm"));

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let app = App::builder().middleware(Trace).build()?;

    app.load(component(DEEPL, "deepl")?).await?;
    app.load(component(WRITER, "writer")?).await?;
    let writer = app.get::<writer::Writer>("writer")?;
    draft(&writer, "deepl").await?;

    let google = component(GOOGLE, "google")?;
    match app.load(google.clone()).await {
        Err(error @ LoadError::WouldMakeAmbiguous { .. }) => {
            println!("google refused: {error}");
        }
        Err(error) => return Err(error.into()),
        Ok(()) => return Err("google unexpectedly loaded".into()),
    }

    app.link("writer", translator::INTERFACE, "deepl")?;
    app.load(google).await?;
    draft(&writer, "linked to deepl").await?;

    app.link("writer", translator::INTERFACE, "google")?;
    draft(&writer, "linked to google").await?;

    app.check()?;
    println!("check: clean");
    Ok(())
}

fn component(bytes: &'static [u8], name: &str) -> Result<Component, Box<dyn Error>> {
    Ok(Component::from_bytes(bytes)?.named(name))
}

async fn draft(writer: &writer::Writer, stage: &str) -> Result<(), Box<dyn Error>> {
    let result = writer.write("hello", "pt").await?;
    match result {
        Ok(text) => println!("{stage}: {text}"),
        Err(error) => println!("{stage}: error: {error}"),
    }
    Ok(())
}
