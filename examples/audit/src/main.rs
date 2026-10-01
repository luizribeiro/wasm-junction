//! Audits a component call with a host resource, byte stream, and per-call data.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
mod trace;

use std::error::Error;
use std::sync::{Arc, Mutex, MutexGuard};

use bindings::{audit, runner};
use trace::Trace;
use wasm_junction::{App, CallContext, CallError, Component, InputStream};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/audit.wasm"));

struct RequestId(u64);

struct Session {
    user: String,
}

#[derive(Default)]
struct Audit {
    lines: Mutex<Vec<String>>,
}

impl Audit {
    fn lines(&self) -> MutexGuard<'_, Vec<String>> {
        self.lines.lock().unwrap()
    }
}

impl audit::Host for Audit {
    type Session = Session;

    fn session_new(&self, _cx: &CallContext, user: String) -> Result<Session, CallError> {
        Ok(Session { user })
    }

    fn session_user(&self, _cx: &CallContext, session: &Session) -> Result<String, CallError> {
        Ok(session.user.clone())
    }

    async fn audit(&self, cx: &CallContext, mut lines: InputStream) -> Result<(), CallError> {
        let request = cx
            .extensions()
            .get::<RequestId>()
            .map_or_else(|| "?".to_owned(), |request| request.0.to_string());
        loop {
            let bytes = match lines.read().await {
                Ok(Some(bytes)) => bytes,
                Ok(None) => break,
                Err(error) => {
                    self.lines()
                        .push(format!("[request {request}] log truncated: {error}"));
                    break;
                }
            };
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) => {
                    self.lines()
                        .push(format!("[request {request}] log truncated: {error}"));
                    break;
                }
            };
            for line in text.lines() {
                self.lines().push(format!("[request {request}] {line}"));
            }
        }
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let audit = Arc::new(Audit::default());
    let app = App::builder()
        .provide(audit::provider(audit.clone()))
        .middleware(Trace::default())
        .build()?;
    app.load(Component::from_bytes(COMPONENT)?.named("audit-log"))
        .await?;

    let runner = app.get::<runner::Runner>("audit-log")?.with(RequestId(42));
    runner.run("Ada").await?;

    for line in audit.lines().iter() {
        println!("stored {line}");
    }
    Ok(())
}
