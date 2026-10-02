//! A deterministic host for a clock-reading WASI component.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod middleware;

use std::error::Error;

use middleware::{FixedClock, Trace};
use wasm_junction::{App, Component, Val, WasiConfig, wasi};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/clock.wasm"));
const CLOCK: &str = "example:clock/clock@0.1.0";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let app = App::builder()
        .provide(wasi::provider())
        .wasi(WasiConfig::new().env("GREETING", "Hello from WASI"))
        .middleware(Trace)
        .middleware(FixedClock)
        .build()?;
    app.load(Component::from_bytes(COMPONENT)?.named("clock"))
        .await?;

    let values = app.call("clock", CLOCK, "run", Vec::new()).await?;
    let [Val::String(output)] = values.as_slice() else {
        return Err("clock guest returned the wrong shape".into());
    };
    println!("{output}");
    Ok(())
}
