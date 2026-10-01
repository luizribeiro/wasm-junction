//! Cleanup behavior for guest-owned host resources.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CallErrorKind, Component, Handle, Provided,
    Provider, Resource, ResourceTable, Val, Vals,
};
use wasm_junction_conformance::{RESOURCE_CLIENT, RESOURCE_HOST, resource_component};
use wasm_junction_wasmtime::WasmtimeEngine;

struct ThreadWake(std::thread::Thread);

#[derive(Clone, Copy)]
struct DropMarker(u32);

#[derive(Debug, PartialEq, Eq)]
struct DropAttempt {
    resource: u32,
    caller: String,
    marker: Option<u32>,
}

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

#[derive(Clone)]
struct FailingDropHost(Arc<FailingDropState>);

struct FailingDropState {
    sessions: ResourceTable<String>,
    attempts: Arc<Mutex<Vec<DropAttempt>>>,
    active: AtomicUsize,
}

impl Default for FailingDropHost {
    fn default() -> Self {
        Self(Arc::new(FailingDropState {
            sessions: ResourceTable::new(RESOURCE_HOST, "session"),
            attempts: Arc::default(),
            active: AtomicUsize::new(0),
        }))
    }
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
        self.0
            .sessions
            .take(&Resource::owned(RESOURCE_HOST, "session", 0))
            .unwrap();
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }

    fn active_resources(&self) -> usize {
        self.0.active.load(Ordering::Relaxed)
    }
}

impl Provider for FailingDropHost {
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
                    Ok(vec![Val::from(
                        self.0
                            .sessions
                            .with(session, |user| format!("profile:{user}"))?,
                    )])
                }
                function => Err(CallError::unavailable(format!(
                    "failing resource host has no `{function}` function"
                ))),
            }
        })
    }

    fn drop_resource(&self, cx: &CallContext, resource: Resource) -> Result<(), CallError> {
        self.0.attempts.lock().unwrap().push(DropAttempt {
            resource: resource.id(),
            caller: cx.caller().to_string(),
            marker: cx.extensions().get::<DropMarker>().map(|marker| marker.0),
        });
        if resource.id() == 0 {
            Err(CallError::trap("drop refused for session#0"))
        } else {
            drop(self.0.sessions.take(&resource)?);
            self.0.active.fetch_sub(1, Ordering::Relaxed);
            Ok(())
        }
    }
}

#[test]
fn successful_calls_report_cleanup_failures_after_attempting_every_drop() {
    let provider = FailingDropHost::default();
    let handle = Handle::new(provider.app(), Arc::from("resource-client")).with(DropMarker(42));
    let error = block_on(handle.call(RESOURCE_CLIENT, "retain", Vec::new())).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Trap);
    assert!(error.to_string().contains("drop refused for session#0"));
    assert_eq!(
        *provider.0.attempts.lock().unwrap(),
        [
            DropAttempt {
                resource: 0,
                caller: "resource-client".to_owned(),
                marker: Some(42),
            },
            DropAttempt {
                resource: 1,
                caller: "resource-client".to_owned(),
                marker: Some(42),
            }
        ]
    );
    assert_eq!(provider.active_resources(), 1);
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
    assert_eq!(
        *provider.0.attempts.lock().unwrap(),
        [
            DropAttempt {
                resource: 0,
                caller: "resource-client".to_owned(),
                marker: None,
            },
            DropAttempt {
                resource: 1,
                caller: "resource-client".to_owned(),
                marker: None,
            }
        ]
    );
    assert_eq!(provider.active_resources(), 1);
    provider.remove_failed_resource();
}
