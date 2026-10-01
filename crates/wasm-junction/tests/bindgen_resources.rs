//! Generated host-resource binding surfaces.

#![forbid(unsafe_code)]

mod support;

use std::sync::{Arc, Mutex};

use wasm_junction::{CallContext, Resource, TypedCall, Val};

wasm_junction::bindgen!({ path: "tests/fixtures/resources/wit" });

#[derive(Debug, PartialEq, Eq)]
struct SessionState(String);

#[derive(Default)]
struct ResourceHost {
    dropped: Mutex<Vec<String>>,
}

impl resources::Host for ResourceHost {
    type Session = SessionState;
    type Host_ = ();
    type Provider = ();

    fn session_new(&self, _cx: &CallContext, user: String) -> SessionState {
        SessionState(user)
    }

    async fn session_profile(&self, _cx: &CallContext, session: &SessionState) -> String {
        std::future::ready(()).await;
        format!("profile:{}", session.0)
    }

    fn session_new_(&self, _cx: &CallContext, session: &SessionState) -> String {
        format!("new:{}", session.0)
    }

    fn session_lookup(&self, _cx: &CallContext, user: String) -> Option<SessionState> {
        Some(SessionState(user))
    }

    fn consume(&self, _cx: &CallContext, value: SessionState) -> String {
        value.0
    }

    fn maybe(&self, _cx: &CallContext, value: Option<SessionState>) -> Option<SessionState> {
        value
    }

    fn choose(
        &self,
        _cx: &CallContext,
        value: Result<SessionState, String>,
    ) -> Result<SessionState, String> {
        value
    }

    fn drop_session(&self, _cx: &CallContext, value: SessionState) {
        self.dropped.lock().unwrap().push(value.0);
    }
}

#[test]
fn host_resources_use_associated_values_and_arc_forwarding() {
    let host = Arc::new(ResourceHost::default());
    let context = CallContext::for_test("plugin");
    let session = resources::Host::session_new(&host, &context, "Ada".to_owned());
    assert_eq!(
        support::block_on(resources::Host::session_profile(&host, &context, &session)),
        "profile:Ada"
    );
    assert_eq!(
        resources::Host::session_new_(&host, &context, &session),
        "new:Ada"
    );
    resources::Host::drop_session(&host, &context, session);
    assert_eq!(*host.dropped.lock().unwrap(), ["Ada"]);
    let _provided = resources::provider(host);
}

#[test]
fn typed_surfaces_keep_resource_ids() {
    let resource = Resource::owned(resources::INTERFACE, "session", 7);
    let call = resources::Consume {
        value: resource.clone(),
    };
    assert_eq!(call.into_vals(), [Val::Resource(resource.clone())]);
    assert_eq!(
        resources::Envelope::try_from(Val::from(resources::Envelope {
            value: resource.clone(),
        }))
        .unwrap()
        .value,
        resource
    );
}
