use std::sync::Arc;

use wasm_junction::{
    BoxFuture, Call, CallContext, CallError, Provided, Provider, ResourceTable, Val, Vals,
};

use crate::RESOURCE_HOST;

/// Hand-written host-resource provider used by engine conformance tests.
#[derive(Clone)]
pub struct ResourceHost(Arc<ResourceProvider>);

impl Default for ResourceHost {
    fn default() -> Self {
        Self(Arc::new(ResourceProvider {
            sessions: ResourceTable::new(RESOURCE_HOST, "session"),
        }))
    }
}

impl ResourceHost {
    /// Wraps this host as the resource fixture's provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        Provided::new(RESOURCE_HOST, self)
    }
}

struct ResourceProvider {
    sessions: ResourceTable<String>,
}

impl Provider for ResourceHost {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        Box::pin(async move {
            match call.function.as_ref() {
                "[constructor]session" => {
                    let [Val::String(user)] = call.args.as_slice() else {
                        return Err(CallError::trap("session constructor expects a user"));
                    };
                    Ok(vec![Val::Resource(self.0.sessions.insert(user.clone())?)])
                }
                "[method]session.profile" => {
                    let [Val::Resource(session)] = call.args.as_slice() else {
                        return Err(CallError::trap("session.profile expects a session"));
                    };
                    Ok(vec![Val::String(
                        self.0
                            .sessions
                            .with(session, |user| format!("profile:{user}"))?,
                    )])
                }
                function => Err(CallError::unavailable(format!(
                    "resource host has no `{function}` function"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use wasm_junction::{Call, CallContext, Caller, Provider, Resource};

    use super::*;

    fn ready<F: Future>(future: F) -> F::Output {
        let future = std::pin::pin!(future);
        match future.poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("provider unexpectedly suspended"),
        }
    }

    #[test]
    fn provider_keeps_sessions_across_calls_and_checks_their_type() {
        let provider = ResourceHost::default();
        let context = CallContext::for_test("resource-client");
        let constructor = Call::new(
            Caller::Component(Arc::from("resource-client")),
            "host",
            RESOURCE_HOST,
            "[constructor]session",
            vec![Val::from("Ada")],
        );
        let session = ready(provider.call(&context, constructor)).unwrap();
        let method = Call::new(
            Caller::Component(Arc::from("resource-client")),
            "host",
            RESOURCE_HOST,
            "[method]session.profile",
            session,
        );
        assert_eq!(
            ready(provider.call(&context, method)).unwrap(),
            [Val::from("profile:Ada")]
        );

        let wrong = Resource::borrowed(RESOURCE_HOST, "file", 0);
        assert!(provider.0.sessions.with(&wrong, String::len).is_err());
    }
}
