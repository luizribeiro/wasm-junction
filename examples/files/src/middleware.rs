//! Trace and preopen policy local to the filesystem example.

use wasm_junction::{Call, CallError, Caller, Middleware, Next, Val, Vals};

const FILESYSTEM_TYPES: &str = "wasi:filesystem/types@0.2.12";
const OPEN_AT: &str = "[method]descriptor.open-at";

/// Prints exported calls and the gated filesystem calls that obtain files.
pub struct Trace;

impl Middleware for Trace {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if matches!(call.caller, Caller::Host)
            || call.interface.as_ref() == "wasi:filesystem/preopens@0.2.12"
            || (call.interface.as_ref() == FILESYSTEM_TYPES && call.function.as_ref() == OPEN_AT)
        {
            println!("call {call}()");
        }
        next.run(call).await
    }
}

/// Refuses file opens descended from one configured guest preopen.
pub struct PreopenPolicy {
    denied: &'static str,
}

impl PreopenPolicy {
    /// Creates a policy that refuses one guest-visible preopen path.
    pub const fn deny(guest_path: &'static str) -> Self {
        Self { denied: guest_path }
    }
}

impl Middleware for PreopenPolicy {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() != FILESYSTEM_TYPES || call.function.as_ref() != OPEN_AT {
            return next.run(call).await;
        }
        let Some(Val::String(preopen)) = call.args.last() else {
            return Err(CallError::trap("open-at has no preopen context"));
        };
        if preopen == self.denied {
            println!("policy deny preopen {preopen}");
            Err(CallError::refused("preopen is blocked"))
        } else {
            println!("policy allow preopen {preopen}");
            next.run(call).await
        }
    }
}
