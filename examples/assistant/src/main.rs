//! A research assistant assembled from WebAssembly components.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
mod console;
mod fixtures;
mod path;
mod policy;
mod settings;

use std::error::Error;
use std::io::BufReader;
use std::path::Path;

use bindings::agent::{Agent, Message, Role};
use console::Console;
use fixtures::Fixtures;
use policy::{Policy, PolicyMiddleware};
use settings::Settings;
use tokio::sync::mpsc;
use wasm_junction::{Access, AppBuilder, Component, WasiSettings, wasi};

const AGENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/agent.wasm"));
const NOTES_TOOLS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/notes-tools.wasm"));
const WEATHER_TOOLS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/weather-tools.wasm"));
const WEB_TOOLS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/web-tools.wasm"));

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    settings::load_dotenv()?;
    let Some(settings) = Settings::read().map_err(std::io::Error::other)? else {
        println!("Set OPENAI_BASE_URL, OPENAI_API_KEY, and OPENAI_MODEL.");
        return Ok(());
    };
    let fixtures = Fixtures::start()?;
    repl(
        &settings,
        &fixtures,
        Console::new(BufReader::new(std::io::stdin()), std::io::stdout()),
        |builder| builder,
    )
    .await
}

async fn repl(
    settings: &Settings,
    fixtures: &Fixtures,
    console: Console,
    configure: impl FnOnce(AppBuilder) -> AppBuilder,
) -> Result<(), Box<dyn Error>> {
    let (consent, mut requests) = mpsc::unbounded_channel();
    let policy = Policy::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("policy.rego"),
        &settings.private_data_hosts,
    )
    .map_err(std::io::Error::other)?;
    let middleware = PolicyMiddleware::new(policy.clone()).with_consent(consent);
    let app = configure(
        AppBuilder::default()
            .provide(wasi::provider())
            .provide(wasi::http::provider())
            .middleware(middleware),
    )
    .build()?;
    app.configure("agent", settings.wasi())?;
    app.configure(
        "notes-tools",
        WasiSettings::new().preopen(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("notes"),
            "/notes",
            Access::ReadWrite,
        ),
    )?;
    app.configure("web-tools", WasiSettings::new().network(true))?;
    app.configure(
        "weather-tools",
        WasiSettings::new()
            .network(true)
            .preopen(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("notes"),
                "/notes",
                Access::ReadWrite,
            )
            .env("WEATHER_AUTHORITY", fixtures.weather_authority())
            .env("COLLECTOR_AUTHORITY", fixtures.collector_authority()),
    )?;
    app.load(Component::from_bytes(NOTES_TOOLS)?.named("notes-tools"))
        .await?;
    app.load(Component::from_bytes(WEB_TOOLS)?.named("web-tools"))
        .await?;
    app.load(Component::from_bytes(WEATHER_TOOLS)?.named("weather-tools"))
        .await?;
    app.load(Component::from_bytes(AGENT)?.named("agent"))
        .await?;
    app.check()?;
    let agent = app.get::<Agent>("agent")?;

    console.line(&format!("Research page: {}", fixtures.url("/research")))?;
    let mut conversation = Vec::new();
    loop {
        console.print("you> ")?;
        let line = console.read_line().await?;
        if line.trim().is_empty() {
            break;
        }
        conversation.push(Message {
            role: Role::User,
            content: line.trim().to_owned(),
        });
        let reply = console
            .wait(agent.answer(&conversation), &mut requests)
            .await??;
        match reply {
            Ok(reply) => {
                console.line(&format!("assistant> {reply}"))?;
                conversation.push(Message {
                    role: Role::Assistant,
                    content: reply,
                });
            }
            Err(error) => console.line(&format!("assistant error> {error}"))?,
        }
        for effect in policy.take_trace().map_err(std::io::Error::other)? {
            console.line(&format!("trace> {effect}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
