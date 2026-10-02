//! Routes one component's import between two translator components.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
mod trace;

use std::error::Error;
use std::sync::Arc;

use bindings::{translator, writer};
use trace::Trace;
use wasm_junction::{App, Component, LoadError};

const DEEPL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/deepl.wasm"));
const GOOGLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/google.wasm"));
const WRITER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/writer.wasm"));

type Output = Arc<dyn Fn(String) + Send + Sync>;

/// Loads, links, and calls the translation components while writing their transcript.
///
/// # Errors
///
/// Returns an error when the application cannot be built, loaded, linked, or called.
pub async fn run(write: impl Fn(String) + Send + Sync + 'static) -> Result<(), Box<dyn Error>> {
    let output: Output = Arc::new(write);
    let app = App::builder()
        .middleware(Trace::new(output.clone()))
        .build()?;

    app.load(component(DEEPL, "deepl")?).await?;
    app.load(component(WRITER, "writer")?).await?;
    let writer = app.get::<writer::Writer>("writer")?;
    draft(&writer, "deepl", &output).await?;

    let google = component(GOOGLE, "google")?;
    match app.load(google.clone()).await {
        Err(error @ LoadError::WouldMakeAmbiguous { .. }) => {
            output(format!("google refused: {error}"));
        }
        Err(error) => return Err(error.into()),
        Ok(()) => return Err("google unexpectedly loaded".into()),
    }

    app.link("writer", translator::INTERFACE, "deepl")?;
    app.load(google).await?;
    draft(&writer, "linked to deepl", &output).await?;

    app.link("writer", translator::INTERFACE, "google")?;
    draft(&writer, "linked to google", &output).await?;

    app.check()?;
    output("check: clean".to_owned());
    Ok(())
}

fn component(bytes: &'static [u8], name: &str) -> Result<Component, Box<dyn Error>> {
    Ok(Component::from_bytes(bytes)?.named(name))
}

async fn draft(
    writer: &writer::Writer,
    stage: &str,
    output: &Output,
) -> Result<(), Box<dyn Error>> {
    let result = writer.write("hello", "pt").await?;
    match result {
        Ok(text) => output(format!("{stage}: {text}")),
        Err(error) => output(format!("{stage}: error: {error}")),
    }
    Ok(())
}
