use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
            active: AtomicUsize::new(0),
        }))
    }
}

impl ResourceHost {
    /// Wraps this host as the resource fixture's provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        Provided::new(RESOURCE_HOST, self)
    }

    /// Returns the number of session values still owned by guests.
    #[must_use]
    pub fn active_resources(&self) -> usize {
        self.0.active.load(Ordering::Relaxed)
    }

    /// Reads a session directly to verify stale-id failures in conformance tests.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when `id` has been dropped or was never allocated.
    pub fn profile(&self, id: u32) -> Result<String, CallError> {
        let resource = wasm_junction::Resource::borrowed(RESOURCE_HOST, "session", id);
        self.0
            .sessions
            .with(&resource, |user| format!("profile:{user}"))
    }
}

struct ResourceProvider {
    sessions: ResourceTable<String>,
    active: AtomicUsize,
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
                    let resource = self.0.sessions.insert(user.clone())?;
                    self.0.active.fetch_add(1, Ordering::Relaxed);
                    Ok(vec![Val::Resource(resource)])
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

    fn drop_resource(&self, resource: wasm_junction::Resource) -> Result<(), CallError> {
        drop(self.0.sessions.take(&resource)?);
        self.0.active.fetch_sub(1, Ordering::Relaxed);
        Ok(())
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
