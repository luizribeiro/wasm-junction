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
    load_dotenv()?;
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

fn load_dotenv() -> Result<(), dotenvy::Error> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    match dotenvy::from_path(path) {
        Ok(()) => Ok(()),
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::{Server, Settings, repl};
    use crate::bindings::{notes_tools, web_tools};
    use wasm_junction::{Call, CallError, Middleware, Next, Vals};

    #[derive(Clone, Default)]
    struct Reached {
        notes_tools: Arc<AtomicBool>,
        web_tools: Arc<AtomicBool>,
    }

    impl Middleware for Reached {
        async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
            if call.interface.as_ref() == notes_tools::INTERFACE {
                self.notes_tools.store(true, Ordering::Relaxed);
            }
            if call.interface.as_ref() == web_tools::INTERFACE {
                self.web_tools.store(true, Ordering::Relaxed);
            }
            next.run(call).await
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn chats_with_configured_endpoint() {
        if std::env::var_os("OPENAI_BASE_URL").is_none() {
            eprintln!("skipped: set OPENAI_BASE_URL to run the assistant end-to-end test");
            return;
        }
        let settings = Settings::read().unwrap().unwrap();
        let server = Server::start().unwrap();
        let input = format!(
            "Use your tools to read research.md and fetch {}, then summarize both.\n\n",
            server.url("/research")
        );
        let reached = Reached::default();
        let observer = reached.clone();
        let mut output = Vec::new();
        repl(
            &settings,
            &server,
            Cursor::new(input),
            &mut output,
            |builder| builder.middleware(observer),
        )
        .await
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        let reply = output
            .split_once("assistant> ")
            .and_then(|(_, reply)| reply.lines().next())
            .unwrap();
        assert!(!reply.trim().is_empty());
        assert!(reached.notes_tools.load(Ordering::Relaxed));
        assert!(reached.web_tools.load(Ordering::Relaxed));
    }
}
