//! Reloads a component while one of its calls is suspended.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
mod gate;
mod trace;

use std::error::Error;
use std::future::{Future, poll_fn};
use std::task::Poll;

use bindings::greeter;
use gate::Gate;
use trace::Trace;
use wasm_junction::{App, Component};

const V1: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/greeter-v1.wasm"));
const V2: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/greeter-v2.wasm"));

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let gate = Gate::default();
    let app = App::builder()
        .provide(gate.clone().provided())
        .middleware(Trace)
        .build()?;
    app.load(Component::from_bytes(V1)?.named("greeter"))
        .await?;
    let greeter = app.get::<greeter::Greeter>("greeter")?;
    println!("first: {}", greeter.greet("Ada").await?);

    println!("starting slow call");
    let slow = greeter.greet_slow("Grace");
    let mut slow = std::pin::pin!(slow);
    poll_fn(|context| match slow.as_mut().poll(context) {
        Poll::Pending if gate.entered() => Poll::Ready(()),
        Poll::Pending => Poll::Pending,
        Poll::Ready(result) => panic!("slow call ended before reload: {result:?}"),
    })
    .await;
    println!("slow call is waiting on v1");

    app.reload("greeter", Component::from_bytes(V2)?).await?;
    println!("after reload: {}", greeter.greet("Lin").await?);

    gate.release();
    println!("slow call finished: {}", slow.await?);
    Ok(())
}
