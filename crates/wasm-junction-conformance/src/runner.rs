use std::error::Error;
use std::fmt::{self, Display};
use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::task::Poll;

use wasm_junction::{
    App, CallError, CallErrorKind, Component, Engine, ImportDispatcher, InvocationContext,
    Resource, Val, Vals,
};

use crate::host::summary;
use crate::{
    ComponentSettings, EXPECTED_RELOAD_TRACE, EXPECTED_RESOURCE_REFUSAL_TRACE,
    EXPECTED_RESOURCE_TRACE, EXPECTED_ROUTED_TRACE, EXPECTED_STREAM_TRACE, EXPECTED_TRACE,
    FixtureHost, RELOAD_GREETER, RELOAD_WRITER, RESOURCE_CLIENT, RESOURCE_HOST, ReloadGreeter,
    ReloadHost, ResourceHost, RoutedHost, STREAM_PROBE, SUMMARIZER, SessionId, StreamHost, Trace,
    component, reload_breaking_component, reload_v1_component, reload_v2_component,
    reload_writer_component, resource_component, stream_component, summarizer,
    translator_component, writer, writer_component,
};

/// A loaded conformance fixture available for additional engine assertions.
pub struct Fixture {
    app: App,
    host: FixtureHost,
    trace: Trace,
}

/// A loaded pair of components for routed-call assertions.
pub struct RoutedFixture {
    app: App,
    writer: writer::Writer,
    host: RoutedHost,
    trace: Trace,
}

/// A loaded host-resource component and its provider.
pub struct ResourceFixture {
    app: App,
    host: ResourceHost,
    trace: Trace,
}

/// A loaded byte-stream component and its hand-written provider.
pub struct StreamFixture {
    app: App,
    host: StreamHost,
    trace: Trace,
}

impl StreamFixture {
    /// Builds and loads the stream guest with its host and tracer.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError`] if application construction, inspection, or loading fails.
    pub async fn new(engine: impl Engine + 'static) -> Result<Self, FixtureError> {
        let host = StreamHost::default();
        let trace = Trace::default();
        let app = App::builder()
            .engine(engine)
            .provide(host.clone().provided())
            .middleware(trace.clone())
            .build()
            .map_err(FixtureError::source)?;
        app.load(
            Component::from_bytes(stream_component())
                .map_err(FixtureError::source)?
                .named("streams"),
        )
        .await
        .map_err(FixtureError::source)?;
        Ok(Self { app, host, trace })
    }

    /// Invokes one exported stream fixture function.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when middleware, the guest, or its host import traps.
    pub async fn call(&self, function: &str, args: Vals) -> Result<Vals, CallError> {
        self.app.call("streams", STREAM_PROBE, function, args).await
    }
}

impl ResourceFixture {
    /// Builds and loads the resource guest with its host and tracer.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError`] if application construction, inspection, or loading fails.
    pub async fn new(engine: impl Engine + 'static) -> Result<Self, FixtureError> {
        let host = ResourceHost::default();
        let trace = Trace::default();
        let app = App::builder()
            .engine(engine)
            .provide(host.clone().provided())
            .middleware(trace.clone())
            .build()
            .map_err(FixtureError::source)?;
        let component = Component::from_bytes(resource_component())
            .map_err(FixtureError::source)?
            .named("resource-client");
        app.load(component).await.map_err(FixtureError::source)?;
        Ok(Self { app, host, trace })
    }

    /// Invokes the guest's resource lifecycle scenario.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when middleware, the guest, or its host import traps.
    pub async fn run(&self, trap: bool) -> Result<Vals, CallError> {
        self.app
            .call(
                "resource-client",
                RESOURCE_CLIENT,
                "run",
                vec![Val::Bool(trap)],
            )
            .await
    }

    async fn profile_dropped(&self, id: u32) -> Result<Vals, CallError> {
        ImportDispatcher::call(
            &self.app,
            InvocationContext::default(),
            Arc::from("resource-client"),
            Arc::from(RESOURCE_HOST),
            Arc::from("[method]session.profile"),
            vec![Val::Resource(Resource::borrowed(
                RESOURCE_HOST,
                "session",
                id,
            ))],
        )
        .await
    }
}

impl RoutedFixture {
    /// Builds and loads the writer and translator with their host and tracer.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError`] if application construction, inspection, or loading fails.
    pub async fn new(engine: impl Engine + 'static) -> Result<Self, FixtureError> {
        let host = RoutedHost::default();
        let trace = Trace::default();
        let app = App::builder()
            .engine(engine)
            .provide(host.clone().provided())
            .middleware(trace.clone())
            .build()
            .map_err(FixtureError::source)?;
        let translator = Component::from_bytes(translator_component())
            .map_err(FixtureError::source)?
            .named("translator");
        let writer = Component::from_bytes(writer_component())
            .map_err(FixtureError::source)?
            .named("writer");
        app.load_all([translator, writer])
            .await
            .map_err(FixtureError::source)?;
        let writer = app
            .get::<writer::Writer>("writer")
            .map_err(FixtureError::source)?;
        Ok(Self {
            app,
            writer,
            host,
            trace,
        })
    }

    /// Invokes the writer's plain or async function.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when middleware or any hop fails.
    pub async fn write(&self, function: &str, text: &str) -> Result<String, CallError> {
        let writer = self.writer.with(SessionId(42));
        match function {
            "write" => writer.write(text).await,
            "write-async" => writer.write_async(text).await,
            _ => Err(CallError::unavailable(format!(
                "writer has no `{function}` function"
            ))),
        }
    }

    /// Returns the host provider called by the translator.
    #[must_use]
    pub const fn host(&self) -> &RoutedHost {
        &self.host
    }

    /// Returns the fixture's tracing middleware.
    #[must_use]
    pub const fn trace(&self) -> &Trace {
        &self.trace
    }
}

impl Fixture {
    /// Builds and loads the notes-summary component with its host and tracer.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError`] if application construction, component inspection, or loading
    /// fails.
    pub async fn new(engine: impl Engine + 'static) -> Result<Self, FixtureError> {
        Self::build(App::builder().engine(engine)).await
    }

    /// Builds and loads the notes-summary component with the target's default engine.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError`] if no default engine is enabled or setup and loading fail.
    pub async fn with_default_engine() -> Result<Self, FixtureError> {
        Self::build(App::builder()).await
    }

    async fn build(builder: wasm_junction::AppBuilder) -> Result<Self, FixtureError> {
        let host = FixtureHost::default();
        let trace = Trace::default();
        let app = builder
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
    /// Returns [`CallError`] when middleware, the guest, or its host import traps.
    pub async fn call(&self, function: &str, args: Vals) -> Result<Vals, CallError> {
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
    check_fixture(Fixture::new(engine).await?).await
}

/// Runs the notes-summary scenario with the target's default engine.
///
/// # Errors
///
/// Returns [`FixtureError`] if setup, invocation, output, or tracing differs from the contract.
pub async fn run_default() -> Result<Fixture, FixtureError> {
    check_fixture(Fixture::with_default_engine().await?).await
}

async fn check_fixture(fixture: Fixture) -> Result<Fixture, FixtureError> {
    let handle = fixture
        .app
        .get::<summarizer::Summarizer>("summarizer")
        .map_err(FixtureError::source)?;
    let result = handle
        .summarize("daily")
        .await
        .map_err(FixtureError::source)?;
    if result != Ok(summary()) {
        return Err(FixtureError::new(format!(
            "unexpected summary result: {result:?}"
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

/// Runs plain and async calls through two components and checks their exact trace.
///
/// # Errors
///
/// Returns [`FixtureError`] if setup, invocation, output, caller, or tracing differs.
pub async fn run_routed(engine: impl Engine + 'static) -> Result<RoutedFixture, FixtureError> {
    let fixture = RoutedFixture::new(engine).await?;
    fixture
        .app
        .configure("writer", ComponentSettings("writer"))
        .map_err(FixtureError::source)?;
    fixture
        .app
        .configure("translator", ComponentSettings("translator"))
        .map_err(FixtureError::source)?;
    for (function, expected) in [
        (
            "write",
            "host[session=42, hop=writer-to-translator]: hello #1",
        ),
        (
            "write-async",
            "host[session=42, hop=writer-to-translator]: async #1",
        ),
    ] {
        let input = function.strip_prefix("write-").unwrap_or("hello");
        let output = fixture
            .write(function, input)
            .await
            .map_err(FixtureError::source)?;
        if output != expected {
            return Err(FixtureError::new(format!("unexpected output: {output}")));
        }
    }
    let callers = fixture.host.callers();
    let translator = wasm_junction::Caller::Component(Arc::from("translator"));
    if callers != [translator.clone(), translator] {
        return Err(FixtureError::new(format!(
            "unexpected callers: {callers:?}"
        )));
    }
    if fixture.host.settings() != [Some("translator"), Some("translator")] {
        return Err(FixtureError::new(format!(
            "unexpected settings: {:?}",
            fixture.host.settings()
        )));
    }
    let expected = EXPECTED_ROUTED_TRACE
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if fixture.trace.entries() != expected {
        return Err(FixtureError::new(format!(
            "unexpected routed trace: {:#?}",
            fixture.trace.entries()
        )));
    }
    Ok(fixture)
}

/// Checks exact host-resource calls and drop events, then cleanup after a guest trap.
///
/// # Errors
///
/// Returns [`FixtureError`] if setup, invocation, tracing, or resource cleanup differs.
pub async fn run_resources(engine: impl Engine + 'static) -> Result<ResourceFixture, FixtureError> {
    let fixture = ResourceFixture::new(engine).await?;
    let output = fixture.run(false).await.map_err(FixtureError::source)?;
    if output != [Val::from("profile:Ada")] {
        return Err(FixtureError::new(format!(
            "unexpected resource output: {output:?}"
        )));
    }
    if fixture.host.active_resources() != 0 {
        return Err(FixtureError::new("normal resource invocation leaked"));
    }
    if fixture.trace.entries() != EXPECTED_RESOURCE_TRACE {
        return Err(FixtureError::new(format!(
            "unexpected resource trace: {:#?}",
            fixture.trace.entries()
        )));
    }
    if fixture.profile_dropped(0).await.is_ok() {
        return Err(FixtureError::new("normal resource invocation leaked"));
    }

    fixture.trace.clear();
    let error = match fixture.run(true).await {
        Ok(output) => {
            return Err(FixtureError::new(format!(
                "resource trap scenario returned {output:?}"
            )));
        }
        Err(error) => error,
    };
    if error.kind() != CallErrorKind::Trap {
        return Err(FixtureError::new(format!(
            "unexpected resource failure: {error}"
        )));
    }
    if fixture.host.active_resources() != 0 {
        return Err(FixtureError::new("trapped resource invocation leaked"));
    }
    if fixture.profile_dropped(1).await.is_ok() {
        return Err(FixtureError::new("trapped resource invocation leaked"));
    }
    Ok(fixture)
}

/// Checks that a failed import defers an owned resource drop to invocation cleanup.
///
/// # Errors
///
/// Returns [`FixtureError`] if the refusal, cleanup, or exact lifecycle trace differs.
pub async fn run_resource_refusal(engine: impl Engine + 'static) -> Result<(), FixtureError> {
    let fixture = ResourceFixture::new(engine).await?;
    let error = match fixture
        .app
        .call(
            "resource-client",
            RESOURCE_CLIENT,
            "drop-after-refusal",
            Vec::new(),
        )
        .await
    {
        Ok(output) => {
            return Err(FixtureError::new(format!(
                "resource refusal scenario returned {output:?}"
            )));
        }
        Err(error) => error,
    };
    if error.kind() != CallErrorKind::Refused || error.to_string() != "resource profile refused" {
        return Err(FixtureError::new(format!(
            "unexpected resource refusal: {error}"
        )));
    }
    if fixture.host.active_resources() != 0 {
        return Err(FixtureError::new("refused resource invocation leaked"));
    }
    if fixture.trace.entries() != EXPECTED_RESOURCE_REFUSAL_TRACE {
        return Err(FixtureError::new(format!(
            "unexpected refused resource trace: {:#?}",
            fixture.trace.entries()
        )));
    }
    Ok(())
}

/// Checks byte streams in both directions and their exact lifecycle trace.
///
/// # Errors
///
/// Returns [`FixtureError`] if setup, invocation, bytes, or tracing differs.
pub async fn run_streams(engine: impl Engine + 'static) -> Result<StreamFixture, FixtureError> {
    let fixture = StreamFixture::new(engine).await?;
    let output = fixture
        .call("motd", Vec::new())
        .await
        .map_err(FixtureError::source)?;
    if output != [Val::from("Have a good day.")] {
        return Err(FixtureError::new(format!("unexpected motd: {output:?}")));
    }
    fixture
        .call("audit", Vec::new())
        .await
        .map_err(FixtureError::source)?;
    if fixture.host.audit() != b"opened note\n" {
        return Err(FixtureError::new("unexpected audit bytes"));
    }
    if fixture.trace.entries() != EXPECTED_STREAM_TRACE {
        return Err(FixtureError::new(format!(
            "unexpected stream trace: {:#?}",
            fixture.trace.entries()
        )));
    }
    Ok(fixture)
}

/// Runs pinned, switched, refused, and forced reload calls with an exact lifecycle trace.
///
/// # Errors
///
/// Returns [`FixtureError`] when setup, reload behavior, output, or tracing differs.
pub async fn run_reload(engine: impl Engine + 'static) -> Result<(), FixtureError> {
    let host = ReloadHost::default();
    let trace = Trace::with_lifecycle();
    let app = App::builder()
        .engine(engine)
        .provide(host.clone().provided())
        .middleware(trace.clone())
        .build()
        .map_err(FixtureError::source)?;
    app.load(reload_component(reload_v1_component(), "greeter")?)
        .await
        .map_err(FixtureError::source)?;
    app.load(reload_component(reload_writer_component(), "writer")?)
        .await
        .map_err(FixtureError::source)?;
    app.link("writer", RELOAD_GREETER, "greeter")
        .map_err(FixtureError::source)?;
    let greeter = app
        .get::<ReloadGreeter>("greeter")
        .map_err(FixtureError::source)?;

    let slow = greeter.greet_slow("Ada");
    let mut slow = std::pin::pin!(slow);
    poll_fn(|context| match slow.as_mut().poll(context) {
        Poll::Pending => Poll::Ready(Ok(())),
        Poll::Ready(result) => Poll::Ready(Err(FixtureError::new(format!(
            "slow call completed before reload: {result:?}"
        )))),
    })
    .await?;
    host.wait_until_entered().await;

    app.reload(
        "greeter",
        reload_component(reload_v2_component(), "unused")?,
    )
    .await
    .map_err(FixtureError::source)?;
    expect_call(
        &app,
        "greeter",
        RELOAD_GREETER,
        "greet",
        "Bob",
        "v2: hello, Bob",
    )
    .await?;
    expect_call(
        &app,
        "writer",
        RELOAD_WRITER,
        "write",
        "Lin",
        "v2: hello, Lin",
    )
    .await?;
    host.release();
    if slow.await.map_err(FixtureError::source)? != "v1: hello, Ada" {
        return Err(FixtureError::new("slow call did not finish on v1"));
    }

    let breaking = reload_component(reload_breaking_component(), "unused")?;
    let Err(error) = app.reload("greeter", breaking.clone()).await else {
        return Err(FixtureError::new("breaking reload unexpectedly succeeded"));
    };
    let message = error.to_string();
    if !message.contains("writer links") || !message.contains("host handle") {
        return Err(FixtureError::new(format!("unnamed dependents: {error}")));
    }
    app.reload_force("greeter", breaking)
        .await
        .map_err(FixtureError::source)?;
    let Err(error) = app
        .call("writer", RELOAD_WRITER, "write", vec![Val::from("Eve")])
        .await
    else {
        return Err(FixtureError::new("dependent call unexpectedly succeeded"));
    };
    if error.kind() != CallErrorKind::Unavailable || !error.to_string().contains("greeter") {
        return Err(FixtureError::new(format!(
            "unclear dependent error: {error}"
        )));
    }
    app.unload_force("greeter")
        .await
        .map_err(FixtureError::source)?;
    if trace.entries() != EXPECTED_RELOAD_TRACE {
        return Err(FixtureError::new(format!(
            "unexpected reload trace: {:#?}",
            trace.entries()
        )));
    }
    Ok(())
}

fn reload_component(bytes: &'static [u8], name: &str) -> Result<Component, FixtureError> {
    Component::from_bytes(bytes)
        .map(|component| component.named(name))
        .map_err(FixtureError::source)
}

async fn expect_call(
    app: &App,
    component: &str,
    interface: &str,
    function: &str,
    input: &str,
    expected: &str,
) -> Result<(), FixtureError> {
    let output = app
        .call(component, interface, function, vec![Val::from(input)])
        .await
        .map_err(FixtureError::source)?;
    if output == [Val::from(expected)] {
        Ok(())
    } else {
        Err(FixtureError::new(format!("unexpected output: {output:?}")))
    }
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
