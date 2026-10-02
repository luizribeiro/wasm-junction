//! A host that provides users to a greeting component.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
mod trace;

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use bindings::{greeter, users};
use trace::Trace;
use wasm_junction::{App, CallContext, CallError, Component};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/greeter.wasm"));

type Output = Arc<dyn Fn(String) + Send + Sync>;

struct Users(HashMap<u32, users::User>);

impl Users {
    fn new() -> Self {
        Self(HashMap::from([
            (1, user("Ada", "en")),
            (2, user("Luiz", "pt")),
            (3, user("Lucía", "es")),
        ]))
    }
}

impl users::Host for Users {
    fn lookup(&self, _cx: &CallContext, id: u32) -> Result<Option<users::User>, CallError> {
        Ok(self.0.get(&id).cloned())
    }
}

fn user(name: &str, language: &str) -> users::User {
    users::User {
        name: name.to_owned(),
        language: language.to_owned(),
    }
}

/// Builds the application, calls the greeter, and writes its complete transcript.
///
/// # Errors
///
/// Returns an error when the application cannot be built, loaded, or called.
pub async fn run(write: impl Fn(String) + Send + Sync + 'static) -> Result<(), Box<dyn Error>> {
    let output: Output = Arc::new(write);
    let app = App::builder()
        .provide(users::provider(Users::new()))
        .middleware(Trace::new(output.clone()))
        .build()?;

    app.load(Component::from_bytes(COMPONENT)?.named("greeter"))
        .await?;
    let greeter = app.get::<greeter::Greeter>("greeter")?;
    for id in [1, 2, 99] {
        match greeter.greet(id).await? {
            Ok(message) => output(format!("result {id}: {message}")),
            Err(message) => output(format!("result {id}: error: {message}")),
        }
    }
    Ok(())
}
