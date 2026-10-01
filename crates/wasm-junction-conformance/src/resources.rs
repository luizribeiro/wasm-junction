use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use wasm_junction::{CallContext, Provided};

wasm_junction::bindgen!({
    path: "resource-wit",
    interfaces: ["example:resources/host@1.0.0"],
});

/// Host-resource implementation used by engine conformance tests.
#[derive(Clone, Default)]
pub struct ResourceHost(Arc<ResourceState>);

#[derive(Default)]
struct ResourceState {
    active: AtomicUsize,
}

impl ResourceHost {
    /// Wraps this host as the resource fixture's provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        host::provider(self)
    }

    /// Returns the number of session values still owned by guests.
    #[must_use]
    pub fn active_resources(&self) -> usize {
        self.0.active.load(Ordering::Relaxed)
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
