//! Cleanup behavior for guest-owned host resources.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CallErrorKind, Component, Provided, Provider,
    Resource, Val, Vals,
};
use wasm_junction_conformance::{RESOURCE_CLIENT, RESOURCE_HOST, ResourceHost, resource_component};
use wasm_junction_wasmtime::WasmtimeEngine;

struct ThreadWake(std::thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[derive(Clone, Default)]
struct FailingDropHost {
    host: ResourceHost,
    attempts: Arc<Mutex<Vec<u32>>>,
}

impl FailingDropHost {
    fn app(&self) -> App {
        let app = App::builder()
            .engine(WasmtimeEngine::new().unwrap())
            .provide(Provided::new(RESOURCE_HOST, self.clone()))
            .build()
            .unwrap();
        let component = Component::from_bytes(resource_component())
            .unwrap()
            .named("resource-client");
        block_on(app.load(component)).unwrap();
        app
    }

    fn remove_failed_resource(&self) {
        self.host
            .drop_resource(Resource::owned(RESOURCE_HOST, "session", 0))
            .unwrap();
    }
}

impl Provider for FailingDropHost {
    fn call<'a>(
        &'a self,
        context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>> {
        self.host.call(context, call)
    }

    fn drop_resource(&self, resource: Resource) -> Result<(), CallError> {
        self.attempts.lock().unwrap().push(resource.id());
        if resource.id() == 0 {
            Err(CallError::trap("drop refused for session#0"))
        } else {
            self.host.drop_resource(resource)
        }
    }
}

#[test]
fn successful_calls_report_cleanup_failures_after_attempting_every_drop() {
    let provider = FailingDropHost::default();
    let error = block_on(provider.app().call(
        "resource-client",
        RESOURCE_CLIENT,
        "retain",
        Vec::new(),
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Trap);
    assert!(error.to_string().contains("drop refused for session#0"));
    assert_eq!(*provider.attempts.lock().unwrap(), [0, 1]);
    assert_eq!(provider.host.active_resources(), 1);
    provider.remove_failed_resource();
}

#[test]
fn traps_include_cleanup_failures_after_attempting_every_drop() {
    let provider = FailingDropHost::default();
    let error = block_on(provider.app().call(
        "resource-client",
        RESOURCE_CLIENT,
        "run",
        vec![Val::Bool(true)],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Trap);
    assert!(error.to_string().contains("wasm `unreachable`"), "{error}");
    assert!(
        error.to_string().contains("drop refused for session#0"),
        "{error}"
    );
    assert_eq!(*provider.attempts.lock().unwrap(), [0, 1]);
    assert_eq!(provider.host.active_resources(), 1);
    provider.remove_failed_resource();
}
