//! A policy-gated outgoing HTTP client with a loopback server.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod middleware;
mod server;

use std::error::Error;

use middleware::{OriginPolicy, Trace};
use server::Server;
use wasm_junction::{App, Component, Val, WasiSettings, wasi};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/http.wasm"));
const CLIENT: &str = "example:http/client@0.1.0";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = Server::start()?;
    let authority = server.authority();
    let app = App::builder()
        .provide(wasi::provider())
        .provide(wasi::http::provider())
        .middleware(Trace)
        .middleware(OriginPolicy::new(authority.clone()))
        .build()?;
    app.configure("http", WasiSettings::new().network(true))?;
    app.load(Component::from_bytes(COMPONENT)?.named("http"))
        .await?;

    let allowed = fetch(&app, &authority, "/message").await?;
    println!("allowed: {allowed}");
    let request = String::from_utf8_lossy(&server.finish()?).to_ascii_lowercase();
    if !request.contains("x-secret: example-token\r\n") || !request.contains("example-body") {
        return Err("server did not receive the rewritten request".into());
    }
    println!("server received rewritten header and request body");

    let denied = fetch(&app, "blocked.invalid", "/message").await?;
    println!("blocked: {denied}");
    Ok(())
}

async fn fetch(app: &App, authority: &str, path: &str) -> Result<String, Box<dyn Error>> {
    let values = app
        .call(
            "http",
            CLIENT,
            "fetch",
            vec![Val::from(authority), Val::from(path)],
        )
        .await?;
    let [Val::String(result)] = values.as_slice() else {
        return Err("HTTP guest returned the wrong shape".into());
    };
    Ok(result.clone())
}
