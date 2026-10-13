//! A research assistant assembled from WebAssembly components.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
pub mod server;

use std::error::Error;
use std::io::{BufRead, Write};
use std::path::Path;

use bindings::agent::{Agent, Message, Role};
use server::Server;
use wasm_junction::{Access, AppBuilder, Component, WasiSettings, wasi};

const AGENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/agent.wasm"));
const NOTES_TOOLS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/notes-tools.wasm"));
const WEB_TOOLS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/web-tools.wasm"));

#[derive(Clone)]
struct Settings {
    scheme: String,
    authority: String,
    path_prefix: String,
    api_key: String,
    model: String,
}

impl Settings {
    fn read() -> Result<Option<Self>, String> {
        let (Ok(base_url), Ok(api_key), Ok(model)) = (
            std::env::var("OPENAI_BASE_URL"),
            std::env::var("OPENAI_API_KEY"),
            std::env::var("OPENAI_MODEL"),
        ) else {
            return Ok(None);
        };
        let (scheme, rest) = base_url
            .split_once("://")
            .ok_or_else(|| "OPENAI_BASE_URL has no scheme".to_owned())?;
        if !matches!(scheme, "http" | "https") {
            return Err("OPENAI_BASE_URL must use HTTP or HTTPS".into());
        }
        let (authority, path_prefix) = rest.find('/').map_or((rest, ""), |at| rest.split_at(at));
        if authority.is_empty() {
            return Err("OPENAI_BASE_URL has no authority".into());
        }
        Ok(Some(Self {
            scheme: scheme.into(),
            authority: authority.into(),
            path_prefix: path_prefix.trim_end_matches('/').into(),
            api_key,
            model,
        }))
    }

    fn wasi(&self) -> WasiSettings {
        WasiSettings::new()
            .network(true)
            .env("OPENAI_SCHEME", &self.scheme)
            .env("OPENAI_AUTHORITY", &self.authority)
            .env("OPENAI_PATH_PREFIX", &self.path_prefix)
            .env("OPENAI_API_KEY", &self.api_key)
            .env("OPENAI_MODEL", &self.model)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let Some(settings) = Settings::read().map_err(std::io::Error::other)? else {
        println!("Set OPENAI_BASE_URL, OPENAI_API_KEY, and OPENAI_MODEL.");
        return Ok(());
    };
    let server = Server::start()?;
    repl(
        &settings,
        &server,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        |builder| builder,
    )
    .await
}

async fn repl(
    settings: &Settings,
    server: &Server,
    mut input: impl BufRead,
    mut output: impl Write,
    configure: impl FnOnce(AppBuilder) -> AppBuilder,
) -> Result<(), Box<dyn Error>> {
    let app = configure(
        AppBuilder::default()
            .provide(wasi::provider())
            .provide(wasi::http::provider()),
    )
    .build()?;
    app.configure("agent", settings.wasi())?;
    app.configure(
        "notes-tools",
        WasiSettings::new().preopen(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("notes"),
            "/notes",
            Access::ReadOnly,
        ),
    )?;
    app.configure("web-tools", WasiSettings::new().network(true))?;
    app.load(Component::from_bytes(NOTES_TOOLS)?.named("notes-tools"))
        .await?;
    app.load(Component::from_bytes(WEB_TOOLS)?.named("web-tools"))
        .await?;
    app.load(Component::from_bytes(AGENT)?.named("agent"))
        .await?;
    app.check()?;
    let agent = app.get::<Agent>("agent")?;

    writeln!(output, "Research page: {}", server.url("/research"))?;
    writeln!(output, "Injected page: {}", server.url("/injected"))?;
    let mut conversation = Vec::new();
    loop {
        write!(output, "you> ")?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        conversation.push(Message {
            role: Role::User,
            content: line.trim().to_owned(),
        });
        let reply = agent
            .answer(&conversation)
            .await?
            .map_err(std::io::Error::other)?;
        writeln!(output, "assistant> {reply}")?;
        conversation.push(Message {
            role: Role::Assistant,
            content: reply,
        });
    }
    Ok(())
}
