//! Cleanup behavior for guest-owned host resources in browsers.

#![cfg(target_family = "wasm")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use wasm_junction::{
    App, BoxFuture, Call, CallContext, CallError, CallErrorKind, Component, Handle, Provided,
    Provider, Resource, ResourceTable, Val, Vals,
};
use wasm_junction_conformance::{RESOURCE_CLIENT, RESOURCE_HOST, resource_component};
use wasm_junction_jco::JcoEngine;

wasm_bindgen_test_configure!(run_in_dedicated_worker);

#[derive(Clone, Copy)]
struct DropMarker(u32);

#[derive(Debug, PartialEq, Eq)]
struct DropAttempt {
    resource: u32,
    caller: String,
    marker: Option<u32>,
}

#[derive(Clone)]
struct FailingDropHost(Rc<FailingDropState>);

struct FailingDropState {
    sessions: ResourceTable<String>,
    attempts: RefCell<Vec<DropAttempt>>,
    active: Cell<usize>,
}

impl Default for FailingDropHost {
    fn default() -> Self {
        Self(Rc::new(FailingDropState {
            sessions: ResourceTable::new(RESOURCE_HOST, "session"),
            attempts: RefCell::default(),
            active: Cell::default(),
        }))
    }
}

impl FailingDropHost {
    async fn app(&self) -> App {
        let app = App::builder()
            .engine(JcoEngine::new())
            .provide(Provided::new(RESOURCE_HOST, self.clone()))
            .build()
            .unwrap();
        app.load(
            Component::from_bytes(resource_component())
                .unwrap()
                .named("resource-client"),
        )
        .await
        .unwrap();
        app
    }

    fn remove_failed_resource(&self) {
        self.0
            .sessions
            .take(&Resource::owned(RESOURCE_HOST, "session", 0))
            .unwrap();
        self.0.active.set(self.0.active.get() - 1);
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
                    self.0.active.set(self.0.active.get() + 1);
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
        self.0.attempts.borrow_mut().push(DropAttempt {
            resource: resource.id(),
            caller: cx.caller().to_string(),
            marker: cx.extensions().get::<DropMarker>().map(|marker| marker.0),
        });
        if resource.id() == 0 {
            Err(CallError::trap("drop refused for session#0"))
        } else {
            drop(self.0.sessions.take(&resource)?);
            self.0.active.set(self.0.active.get() - 1);
            Ok(())
        }
    }
}

#[wasm_bindgen_test]
async fn successful_calls_report_cleanup_failures_after_attempting_every_drop() {
    let provider = FailingDropHost::default();
    let handle = Handle::new(
        provider.app().await,
        Arc::from("resource-client"),
        RESOURCE_CLIENT,
    )
    .with(DropMarker(42));
    let error = handle
        .call(RESOURCE_CLIENT, "retain", Vec::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Trap);
    assert!(error.to_string().contains("drop refused for session#0"));
    assert_eq!(
        *provider.0.attempts.borrow(),
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
    assert_eq!(provider.0.active.get(), 1);
    provider.remove_failed_resource();
}

#[wasm_bindgen_test]
async fn traps_include_cleanup_failures_after_attempting_every_drop() {
    let provider = FailingDropHost::default();
    let error = provider
        .app()
        .await
        .call(
            "resource-client",
            RESOURCE_CLIENT,
            "run",
            vec![Val::Bool(true)],
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Trap);
    let message = error.to_string();
    assert!(
        message.to_ascii_lowercase().contains("unreachable"),
        "{message}"
    );
    assert!(message.contains("drop refused for session#0"));
    assert_eq!(
        *provider.0.attempts.borrow(),
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
    assert_eq!(provider.0.active.get(), 1);
    provider.remove_failed_resource();
}
