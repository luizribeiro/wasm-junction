use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use wasm_junction::{
    BoxFuture, Call, CallContext, CallError, Provided, Provider, ResourceTable, Val, Vals,
};

use crate::RESOURCE_HOST;

wasm_junction::bindgen!({
    path: "resource-wit",
    interfaces: ["example:resources/host@1.0.0"],
});

/// Hand-written host-resource provider used by engine conformance tests.
#[derive(Clone)]
pub struct ResourceHost(Arc<ResourceProvider>);

impl Default for ResourceHost {
    fn default() -> Self {
        Self(Arc::new(ResourceProvider {
            files: ResourceTable::new(RESOURCE_HOST, "file"),
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

    /// Creates a provider-owned session for a host-to-guest export call.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when the resource id space is exhausted.
    pub fn open(&self, user: impl Into<String>) -> Result<wasm_junction::Resource, CallError> {
        self.insert(&self.0.sessions, user.into())
    }

    /// Creates a provider-owned file for resource-type mismatch checks.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when the resource id space is exhausted.
    pub fn open_file(&self, name: impl Into<String>) -> Result<wasm_junction::Resource, CallError> {
        self.insert(&self.0.files, name.into())
    }

    /// Returns an owned session to the provider.
    ///
    /// # Errors
    ///
    /// Returns [`CallError`] when the resource was already returned or dropped.
    pub fn close(&self, resource: &wasm_junction::Resource) -> Result<(), CallError> {
        if resource.name() == "file" {
            drop(self.0.files.take(resource)?);
        } else {
            drop(self.0.sessions.take(resource)?);
        }
        self.0.active.fetch_sub(1, Ordering::Relaxed);
        Ok(())
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

    fn insert(
        &self,
        table: &ResourceTable<String>,
        value: String,
    ) -> Result<wasm_junction::Resource, CallError> {
        let resource = table.insert(value)?;
        self.0.active.fetch_add(1, Ordering::Relaxed);
        Ok(resource)
    }
}

struct ResourceProvider {
    files: ResourceTable<String>,
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
                    let resource = self.open(user.clone())?;
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

    fn drop_resource(
        &self,
        _cx: &CallContext,
        resource: wasm_junction::Resource,
    ) -> Result<(), CallError> {
        self.close(&resource)
    }
}

impl host::Host for ResourceHost {
    type File = String;
    type Session = String;

    fn open_file(&self, _cx: &CallContext, name: String) -> String {
        self.0.active.fetch_add(1, Ordering::Relaxed);
        name
    }

    fn session_new(&self, _cx: &CallContext, user: String) -> String {
        self.0.active.fetch_add(1, Ordering::Relaxed);
        user
    }

    fn session_profile(&self, _cx: &CallContext, session: &String) -> String {
        format!("profile:{session}")
    }

    fn drop_file(&self, _cx: &CallContext, _file: String) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }

    fn drop_session(&self, _cx: &CallContext, _session: String) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_tracks_resource_values_until_drop() {
        let host = ResourceHost::default();
        let context = CallContext::for_test("resource-client");
        let session = host::Host::session_new(&host, &context, "Ada".to_owned());
        assert_eq!(host.active_resources(), 1);
        assert_eq!(
            host::Host::session_profile(&host, &context, &session),
            "profile:Ada"
        );
        host::Host::drop_session(&host, &context, session);
        assert_eq!(host.active_resources(), 0);
    }
}
