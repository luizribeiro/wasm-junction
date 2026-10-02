//! Reloads a component while one of its calls is suspended.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
mod gate;
mod trace;

use std::error::Error;
use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::task::Poll;

use bindings::greeter;
use gate::Gate;
use trace::Trace;
use wasm_junction::{App, Component};

const V1: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/greeter-v1.wasm"));
const V2: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/greeter-v2.wasm"));

type Output = Arc<dyn Fn(String) + Send + Sync>;

/// Reloads the greeter while an earlier call waits and writes the complete transcript.
///
/// # Errors
///
/// Returns an error when the application cannot be built, loaded, reloaded, or called.
///
/// # Panics
///
/// Panics if the deliberately suspended call completes before the reload begins.
pub async fn run(write: impl Fn(String) + Send + Sync + 'static) -> Result<(), Box<dyn Error>> {
    let output: Output = Arc::new(write);
    let gate = Gate::default();
    let app = App::builder()
        .provide(gate.clone().provided())
        .middleware(Trace::new(output.clone()))
        .build()?;
    app.load(Component::from_bytes(V1)?.named("greeter"))
        .await?;
    let greeter = app.get::<greeter::Greeter>("greeter")?;
    output(format!("first: {}", greeter.greet("Ada").await?));

    output("starting slow call".to_owned());
    let slow = greeter.greet_slow("Grace");
    let mut slow = std::pin::pin!(slow);
    poll_fn(|context| match slow.as_mut().poll(context) {
        Poll::Pending if gate.entered() => Poll::Ready(()),
        Poll::Pending => Poll::Pending,
        Poll::Ready(result) => panic!("slow call ended before reload: {result:?}"),
    })
    .await;
    output("slow call is waiting on v1".to_owned());

    app.reload("greeter", Component::from_bytes(V2)?).await?;
    output(format!("after reload: {}", greeter.greet("Lin").await?));

    gate.release();
    output(format!("slow call finished: {}", slow.await?));
    Ok(())
}
