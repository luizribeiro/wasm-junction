//! Generated host-resource binding surfaces.

#![forbid(unsafe_code)]

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use wasm_junction::{
    App, CallContext, CallError, CallErrorKind, Component, ImportDispatcher, InvocationContext,
    Provided, Resource, TypedCall, Val,
};

wasm_junction::bindgen!({ path: "tests/fixtures/resources/wit" });

const PLUGIN_WIT: &str = r"
    package test:resource-plugin@1.0.0;
    interface client {
      use test:resources/resources@1.0.0.{host, session};
      open: func(user: string) -> session;
      profile: func(value: borrow<session>) -> string;
      new: func(value: borrow<session>) -> string;
      lookup: func(user: string) -> option<session>;
      consume: func(value: session) -> string;
      maybe: func(value: option<session>) -> option<session>;
      choose: func(value: result<session, string>) -> result<session, string>;
      make-host: func() -> host;
    }
    world plugin { import test:resources/resources@1.0.0; export client; }
";

#[derive(Debug, PartialEq, Eq)]
struct SessionState(String);

struct DropProbe(Arc<AtomicUsize>);

#[derive(Default)]
struct Gate {
    paused: AtomicUsize,
    released: AtomicUsize,
    waker: Mutex<Option<Waker>>,
}

impl Gate {
    fn pause(&self) {
        self.released.store(0, Ordering::Relaxed);
        self.paused.store(1, Ordering::Relaxed);
    }

    async fn wait(&self) {
        std::future::poll_fn(|cx| {
            if self.paused.load(Ordering::Relaxed) == 0
                || self.released.load(Ordering::Relaxed) == 1
            {
                Poll::Ready(())
            } else {
                *self.waker.lock().unwrap() = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await;
    }

    fn release(&self) {
        self.released.store(1, Ordering::Relaxed);
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct ResourceHost {
    dropped: Mutex<Vec<String>>,
    default_drops: Arc<AtomicUsize>,
    gate: Gate,
}

impl resources::Host for ResourceHost {
    type Session = SessionState;
    type Host_ = DropProbe;
    type Provider = ();

    fn session_new(&self, _cx: &CallContext, user: String) -> Result<SessionState, CallError> {
        Ok(SessionState(user))
    }

    async fn session_profile(
        &self,
        _cx: &CallContext,
        session: &SessionState,
    ) -> Result<String, CallError> {
        self.gate.wait().await;
        if session.0 == "refuse" {
            return Err(CallError::refused("profile is private"));
        }
        Ok(format!("profile:{}", session.0))
    }

    fn session_new_(&self, _cx: &CallContext, session: &SessionState) -> Result<String, CallError> {
        Ok(format!("new:{}", session.0))
    }

    fn session_lookup(
        &self,
        _cx: &CallContext,
        user: String,
    ) -> Result<Option<SessionState>, CallError> {
        Ok(Some(SessionState(user)))
    }

    fn consume(&self, _cx: &CallContext, value: SessionState) -> Result<String, CallError> {
        Ok(value.0)
    }

    fn maybe(
        &self,
        _cx: &CallContext,
        value: Option<SessionState>,
    ) -> Result<Option<SessionState>, CallError> {
        Ok(value)
    }

    fn choose(
        &self,
        _cx: &CallContext,
        value: Result<SessionState, String>,
    ) -> Result<Result<SessionState, String>, CallError> {
        Ok(value)
    }

    fn make_host(&self, _cx: &CallContext) -> Result<DropProbe, CallError> {
        Ok(DropProbe(self.default_drops.clone()))
    }

    fn drop_session(&self, _cx: &CallContext, value: SessionState) -> Result<(), CallError> {
        if value.0 == "refuse-drop" {
            return Err(CallError::refused("session cannot be dropped"));
        }
        self.dropped.lock().unwrap().push(value.0);
        Ok(())
    }
}

#[test]
fn host_resources_use_associated_values_and_arc_forwarding() {
    let host = Arc::new(ResourceHost::default());
    let context = CallContext::for_test("plugin");
    let session = resources::Host::session_new(&host, &context, "Ada".to_owned()).unwrap();
    assert_eq!(
        support::block_on(resources::Host::session_profile(&host, &context, &session)).unwrap(),
        "profile:Ada"
    );
    assert_eq!(
        resources::Host::session_new_(&host, &context, &session).unwrap(),
        "new:Ada"
    );
    resources::Host::drop_session(&host, &context, session).unwrap();
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

#[test]
fn app_dispatch_maps_resource_values_and_stale_ids() {
    let host = Arc::new(ResourceHost::default());
    let app = resource_app(host.clone());
    let call = |function, args| {
        support::block_on(app.call("plugin", support::RESOURCE_BINDGEN_CLIENT, function, args))
    };

    let session = one_resource(&call("open", vec![Val::from("Ada")]).unwrap());
    let borrowed = Resource::borrowed(session.interface(), session.name(), session.id());
    assert_eq!(
        call("profile", vec![Val::Resource(borrowed.clone())]).unwrap(),
        [Val::from("profile:Ada")]
    );
    assert_eq!(
        call("new", vec![Val::Resource(borrowed)]).unwrap(),
        [Val::from("new:Ada")]
    );

    let found = one_optional_resource(&call("lookup", vec![Val::from("Grace")]).unwrap());
    let found = one_optional_resource(
        &call(
            "maybe",
            vec![Val::Option(Some(Box::new(Val::Resource(found))))],
        )
        .unwrap(),
    );
    let found = one_result_resource(
        &call(
            "choose",
            vec![Val::Result(Ok(Some(Box::new(Val::Resource(found)))))],
        )
        .unwrap(),
    );
    assert_eq!(
        call("consume", vec![Val::Resource(found)]).unwrap(),
        [Val::from("Grace")]
    );

    support::block_on(ImportDispatcher::drop_resource(
        &app,
        InvocationContext::default(),
        Arc::from("plugin"),
        session.clone(),
    ))
    .unwrap();
    assert_eq!(*host.dropped.lock().unwrap(), ["Ada"]);
    let stale = Resource::borrowed(session.interface(), session.name(), session.id());
    let error = call("profile", vec![Val::Resource(stale)]).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Trap);
    assert!(error.to_string().contains("unknown resource"));

    let refused = one_resource(&call("open", vec![Val::from("refuse")]).unwrap());
    let error = call(
        "profile",
        vec![Val::Resource(Resource::borrowed(
            refused.interface(),
            refused.name(),
            refused.id(),
        ))],
    )
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "profile is private");

    let refused_drop = one_resource(&call("open", vec![Val::from("refuse-drop")]).unwrap());
    let error = support::block_on(ImportDispatcher::drop_resource(
        &app,
        InvocationContext::default(),
        Arc::from("plugin"),
        refused_drop,
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "session cannot be dropped");

    let host_resource = one_resource(&call("make-host", Vec::new()).unwrap());
    support::block_on(ImportDispatcher::drop_resource(
        &app,
        InvocationContext::default(),
        Arc::from("plugin"),
        host_resource,
    ))
    .unwrap();
    assert_eq!(host.default_drops.load(Ordering::Relaxed), 1);
}

#[test]
fn generated_provider_rejects_the_wrong_resource_type() {
    let app = resource_app(Arc::new(ResourceHost::default()));
    let wrong = Resource::borrowed(resources::INTERFACE, "host", 0);
    let error = support::block_on(app.call(
        "plugin",
        support::RESOURCE_BINDGEN_CLIENT,
        "profile",
        vec![Val::Resource(wrong)],
    ))
    .unwrap_err();

    assert!(error.to_string().contains("expected resource"), "{error}");
    assert!(error.to_string().contains("/host"), "{error}");
}

#[test]
fn generated_provider_reports_resource_id_exhaustion() {
    let provider =
        resources::__provider_with_next_resource_id(Arc::new(ResourceHost::default()), u32::MAX);
    let app = resource_app_with(provider);
    let last = one_resource(
        &support::block_on(app.call(
            "plugin",
            support::RESOURCE_BINDGEN_CLIENT,
            "open",
            vec![Val::from("Ada")],
        ))
        .unwrap(),
    );
    assert_eq!(last.id(), u32::MAX);

    let error = support::block_on(app.call(
        "plugin",
        support::RESOURCE_BINDGEN_CLIENT,
        "open",
        vec![Val::from("Grace")],
    ))
    .unwrap_err();
    assert!(error.to_string().contains("is exhausted"), "{error}");
}

fn resource_app(host: Arc<ResourceHost>) -> App {
    resource_app_with(resources::provider(host))
}

fn resource_app_with(provider: Provided) -> App {
    let app = App::builder()
        .engine(support::FakeEngine)
        .provide(provider)
        .build()
        .unwrap();
    let bytes = support::component_bytes_from(
        &[
            (
                "resources.wit",
                include_str!("fixtures/resources/wit/package.wit"),
            ),
            ("plugin.wit", PLUGIN_WIT),
        ],
        "test:resource-plugin/plugin@1.0.0",
    );
    support::block_on(app.load(Component::from_bytes(bytes).unwrap().named("plugin"))).unwrap();
    app
}

#[test]
fn async_resource_borrows_release_the_table_lock() {
    use std::future::Future;

    let host = Arc::new(ResourceHost::default());
    let app = resource_app(host.clone());
    let session = one_resource(
        &support::block_on(app.call(
            "plugin",
            support::RESOURCE_BINDGEN_CLIENT,
            "open",
            vec![Val::from("Ada")],
        ))
        .unwrap(),
    );
    host.gate.pause();
    let borrowed = Resource::borrowed(session.interface(), session.name(), session.id());
    let mut profile = std::pin::pin!(app.call(
        "plugin",
        support::RESOURCE_BINDGEN_CLIENT,
        "profile",
        vec![Val::Resource(borrowed)],
    ));
    assert!(matches!(
        profile
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));

    let lookup = support::block_on(app.call(
        "plugin",
        support::RESOURCE_BINDGEN_CLIENT,
        "lookup",
        vec![Val::from("Grace")],
    ));
    assert!(lookup.is_ok());
    let error = support::block_on(app.call(
        "plugin",
        support::RESOURCE_BINDGEN_CLIENT,
        "consume",
        vec![Val::Resource(session.clone())],
    ))
    .unwrap_err();
    assert!(error.to_string().contains("while it is borrowed"));

    host.gate.release();
    assert_eq!(
        support::block_on(profile).unwrap(),
        [Val::from("profile:Ada")]
    );
    assert_eq!(
        support::block_on(app.call(
            "plugin",
            support::RESOURCE_BINDGEN_CLIENT,
            "consume",
            vec![Val::Resource(session)],
        ))
        .unwrap(),
        [Val::from("Ada")]
    );
}
fn one_resource(values: &[Val]) -> Resource {
    let [Val::Resource(resource)] = values else {
        panic!("expected resource")
    };
    resource.clone()
}

fn one_optional_resource(values: &[Val]) -> Resource {
    let [Val::Option(Some(value))] = values else {
        panic!("expected optional resource")
    };
    let Val::Resource(resource) = value.as_ref() else {
        panic!("expected optional resource")
    };
    resource.clone()
}

fn one_result_resource(values: &[Val]) -> Resource {
    let [Val::Result(Ok(Some(value)))] = values else {
        panic!("expected result resource")
    };
    let Val::Resource(resource) = value.as_ref() else {
        panic!("expected result resource")
    };
    resource.clone()
}
