//! A host that provides users to a greeting component.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
mod trace;

use std::collections::HashMap;
use std::error::Error;

use bindings::{greeter, users};
use trace::Trace;
use wasm_junction::{App, CallContext, Component};
use wasm_junction_wasmtime::WasmtimeEngine;

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/greeter.wasm"));

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
    fn lookup(&self, _cx: &CallContext, id: u32) -> Option<users::User> {
        self.0.get(&id).cloned()
    }
}

fn user(name: &str, language: &str) -> users::User {
    users::User {
        name: name.to_owned(),
        language: language.to_owned(),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let app = App::builder()
        .engine(WasmtimeEngine::new()?)
        .provide(users::provider(Users::new()))
        .middleware(Trace)
        .build()?;

    app.load(Component::from_bytes(COMPONENT)?.named("greeter"))
        .await?;
    let greeter = app.get::<greeter::Greeter>("greeter")?;
    for id in [1, 2, 99] {
        match greeter.greet(id).await? {
            Ok(message) => println!("result {id}: {message}"),
            Err(message) => println!("result {id}: error: {message}"),
        }
    }
    Ok(())
}
