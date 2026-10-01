//! Tests for WIT names that are Rust keywords.

#![forbid(unsafe_code)]

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use support::{FakeEngine, block_on, component_bytes};

use wasm_junction::{
    App, Call, CallContext, CallError, Caller, Component, InterfaceHandle, Middleware, Next,
    TypedCall, Vals,
};

wasm_junction::bindgen!({ path: "tests/fixtures/keywords/wit" });

struct Names;

impl super_::Host for Names {
    fn host(&self, _cx: &CallContext, cx_: String) -> Result<String, CallError> {
        Ok(cx_)
    }

    fn provider(&self, _cx: &CallContext, call: String) -> Result<String, CallError> {
        Ok(call)
    }

    fn call(
        &self,
        _cx: &CallContext,
        call: String,
        args: String,
        value: String,
        host: String,
        self_: String,
    ) -> Result<String, CallError> {
        Ok([call, args, value, host, self_].join(":"))
    }

    fn super_(&self, _cx: &CallContext) -> Result<String, CallError> {
        Ok("super".into())
    }
}

impl host::Host for Names {
    fn host_handle(&self, _cx: &CallContext, value: String) -> Result<String, CallError> {
        Ok(value)
    }
}

struct ProviderNames;

impl host_provider::Host for ProviderNames {
    fn ping(&self, _cx: &CallContext) -> Result<String, CallError> {
        Ok("pong".to_owned())
    }
}

#[derive(Clone, Default)]
struct RecursiveHost(Arc<Mutex<Option<host::HostHandle>>>);

impl host::Host for RecursiveHost {
    fn host_handle(&self, cx: &CallContext, value: String) -> Result<String, CallError> {
        let handle = self.0.lock().unwrap().clone().unwrap();
        Ok(support::block_on(handle.within(cx).host_handle(&value))
            .unwrap_or_else(|error| error.to_string()))
    }
}

struct CountHostCalls(Arc<AtomicUsize>);

impl Middleware for CountHostCalls {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.callee.as_ref() == "plugin" {
            assert_eq!(call.caller, Caller::Host);
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        next.run(call).await
    }
}

fn requires_handle<T: InterfaceHandle>() {}

#[test]
fn keywords_generate_valid_documented_identifiers() {
    let value = super_::Self_ {
        r#type: "type".to_owned(),
        r#match: true,
        self_: "self".to_owned(),
        super_: "super".to_owned(),
        crate_: "crate".to_owned(),
    };
    let encoded = wasm_junction::Val::from(value.clone());
    assert_eq!(super_::Self_::try_from(encoded).unwrap(), value);
}

#[test]
fn generated_names_do_not_clash_with_the_host_surface() {
    let context = CallContext::for_test("caller");
    let host = super_::Host::host(&Names, &context, "context".into()).unwrap();
    assert_eq!(host, "context");
    let provider = super_::Host::provider(&Names, &context, "call".into()).unwrap();
    assert_eq!(provider, "call");
    assert_eq!(super_::Host::super_(&Names, &context).unwrap(), "super");

    let call = super_::Call {
        call: "call".into(),
        args: "args".into(),
        value: "value".into(),
        host: "host".into(),
        self_: "self".into(),
    };
    let decoded = super_::Call::from_vals(&call.into_vals()).unwrap();
    assert_eq!(
        super_::Host::call(
            &Names,
            &context,
            decoded.call,
            decoded.args,
            decoded.value,
            decoded.host,
            decoded.self_,
        )
        .unwrap(),
        "call:args:value:host:self"
    );
    let _provided = super_::provider(Names);
}

#[test]
fn reserved_interface_names_generate_distinct_handles() {
    let app = App::builder()
        .engine(FakeEngine)
        .provide(host::provider(Names))
        .build()
        .unwrap();
    let component = component_bytes(include_str!("fixtures/keywords/wit/package.wit"), "plugin");
    block_on(app.load(Component::from_bytes(component).unwrap().named("plugin"))).unwrap();

    let handle = app.get::<host::HostHandle>("plugin").unwrap();
    assert_eq!(
        block_on(handle.host_handle("round trip")).unwrap(),
        "round trip"
    );
    let call = host::HostHandle_ {
        value: "call view".to_owned(),
    };
    assert_eq!(
        host::HostHandle_::from_vals(&call.into_vals())
            .unwrap()
            .value,
        "call view"
    );

    requires_handle::<host_provider::HostProviderHandle>();
    let _provided = host_provider::provider(ProviderNames);
}

#[test]
fn host_mediated_cycles_keep_depth_and_host_callers() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = RecursiveHost::default();
    let app = App::builder()
        .engine(FakeEngine)
        .provide(host::provider(provider.clone()))
        .middleware(CountHostCalls(calls.clone()))
        .max_call_depth(3)
        .build()
        .unwrap();
    let component = component_bytes(include_str!("fixtures/keywords/wit/package.wit"), "plugin");
    block_on(app.load(Component::from_bytes(component).unwrap().named("plugin"))).unwrap();
    let handle = app.get::<host::HostHandle>("plugin").unwrap();
    *provider.0.lock().unwrap() = Some(handle.clone());

    let result = block_on(handle.host_handle("cycle")).unwrap();

    assert_eq!(result, "maximum call depth of 3 exceeded");
    assert_eq!(calls.load(Ordering::Relaxed), 4);
}
