use std::error::Error;
use std::fmt::{self, Display};

use wasm_junction::{App, Component, Engine, Trap, Val, Vals};

use crate::{EXPECTED_TRACE, FixtureHost, SUMMARIZER, Trace, component, sample_summary};

/// A loaded conformance fixture available for additional engine assertions.
pub struct Fixture {
    app: App,
    host: FixtureHost,
    trace: Trace,
}

impl Fixture {
    /// Builds and loads the notes-summary component with its host and tracer.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError`] if application construction, component inspection, or loading
    /// fails.
    pub async fn new(engine: impl Engine + 'static) -> Result<Self, FixtureError> {
        let host = FixtureHost::default();
        let trace = Trace::default();
        let app = App::builder()
            .engine(engine)
            .provide(host.clone().provided())
            .middleware(trace.clone())
            .build()
            .map_err(FixtureError::source)?;
        let component = Component::from_bytes(component())
            .map_err(FixtureError::source)?
            .named("summarizer");
        app.load(component).await.map_err(FixtureError::source)?;
        Ok(Self { app, host, trace })
    }

    /// Invokes a fixture export through the application dispatcher.
    ///
    /// # Errors
    ///
    /// Returns [`Trap`] when middleware, the guest, or its host import traps.
    pub async fn call(&self, function: &str, args: Vals) -> Result<Vals, Trap> {
        self.app
            .call("summarizer", SUMMARIZER, function, args)
            .await
    }

    /// Returns the host used by this fixture.
    #[must_use]
    pub const fn host(&self) -> &FixtureHost {
        &self.host
    }

    /// Returns the fixture's tracing middleware.
    #[must_use]
    pub const fn trace(&self) -> &Trace {
        &self.trace
    }
}

/// Runs the successful notes-summary scenario and checks its result and exact trace.
///
/// Engine crates call this from their conformance test, then use the returned fixture for
/// engine-specific assertions.
///
/// # Errors
///
/// Returns [`FixtureError`] if setup, invocation, output, or tracing differs from the contract.
pub async fn run(engine: impl Engine + 'static) -> Result<Fixture, FixtureError> {
    let fixture = Fixture::new(engine).await?;
    let values = fixture
        .call("summarize", vec![Val::from("daily")])
        .await
        .map_err(FixtureError::source)?;
    if values != [sample_summary()] {
        return Err(FixtureError::new(format!(
            "unexpected summary values: {values:?}"
        )));
    }
    let expected = EXPECTED_TRACE
        .iter()
        .map(|entry| (*entry).to_owned())
        .collect::<Vec<_>>();
    if fixture.trace.entries() != expected {
        return Err(FixtureError::new(format!(
            "unexpected trace: {:#?}",
            fixture.trace.entries()
        )));
    }
    Ok(fixture)
}

/// A failure while constructing or running the conformance fixture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureError(String);

impl FixtureError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn source(error: impl Display) -> Self {
        Self::new(error.to_string())
    }
}

impl Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for FixtureError {}
