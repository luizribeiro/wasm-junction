//! Middleware local to the clock example.

use wasm_junction::{Call, CallError, Middleware, Next, Val, Vals};

/// Prints every application and WASI call in invocation order.
pub struct Trace;

impl Middleware for Trace {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        println!("call {call}()");
        next.run(call).await
    }
}

/// Replaces wall-clock reads with a deterministic instant.
pub struct FixedClock;

impl Middleware for FixedClock {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let fixed = call.interface.as_ref() == "wasi:clocks/wall-clock@0.2.12"
            && call.function.as_ref() == "now";
        let result = next.run(call).await?;
        if fixed {
            Ok(vec![Val::Record(vec![
                ("seconds".to_owned(), Val::U64(1_700_000_000)),
                ("nanoseconds".to_owned(), Val::U32(123_456_789)),
            ])])
        } else {
            Ok(result)
        }
    }
}
