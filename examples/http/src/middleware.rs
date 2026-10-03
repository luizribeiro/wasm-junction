//! Trace and origin policy local to this example.

use wasm_junction::{Call, CallError, Caller, Middleware, Next, Val, Vals};

/// Prints application calls and the outgoing request boundary.
pub struct Trace;

impl Middleware for Trace {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if matches!(call.caller, Caller::Host)
            || (call.interface.as_ref() == "wasi:http/client@0.3.0"
                && call.function.as_ref() == "send")
        {
            println!("call {call}()");
        }
        next.run(call).await
    }
}

/// Allows one origin, refuses all others, and adds a server credential.
pub struct OriginPolicy {
    allowed: String,
}

impl OriginPolicy {
    /// Creates a policy for one exact HTTP authority.
    pub fn new(allowed: String) -> Self {
        Self { allowed }
    }
}

impl Middleware for OriginPolicy {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() != "wasi:http/client@0.3.0" || call.function.as_ref() != "send" {
            return next.run(call).await;
        }
        let method = variant_name(&call.args[1], "method")?;
        let authority = optional_string(&call.args[3], "authority")?;
        let path = optional_string(&call.args[4], "path")?;
        let origin = if authority == self.allowed {
            "local"
        } else {
            authority.as_str()
        };
        if authority != self.allowed {
            println!("policy deny {method} http://{origin}{path}");
            return Err(CallError::refused("origin is not allowed"));
        }
        println!("policy allow {method} http://{origin}{path}");
        let Val::List(headers) = &mut call.args[5] else {
            return Err(CallError::trap("send headers had the wrong shape"));
        };
        headers.push(Val::Tuple(vec![
            Val::from("x-secret"),
            Val::from(b"example-token".to_vec()),
        ]));
        next.run(call).await
    }
}

fn variant_name(value: &Val, expected: &str) -> Result<String, CallError> {
    let Val::Variant { case, value: None } = value else {
        return Err(CallError::trap(format!("expected {expected}")));
    };
    Ok(case.to_ascii_uppercase())
}

fn optional_string(value: &Val, expected: &str) -> Result<String, CallError> {
    let Val::Option(Some(value)) = value else {
        return Err(CallError::trap(format!("expected {expected}")));
    };
    let Val::String(value) = value.as_ref() else {
        return Err(CallError::trap(format!("expected {expected}")));
    };
    Ok(value.clone())
}
