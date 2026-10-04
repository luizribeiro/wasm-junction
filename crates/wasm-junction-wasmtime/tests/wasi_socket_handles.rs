//! Socket resource provenance behavior.

#![cfg(feature = "wasi")]

use std::sync::Mutex;

use wasm_junction::{
    App, Call, CallError, Component, Middleware, Next, Resource, Val, Vals, WasiSettings,
};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const EXPORT: &str = "test:wasi/environment@0.1.0";

#[derive(Clone, Copy)]
enum InvalidHandle {
    Foreign,
    Mistyped,
    Unscoped,
    Nonexistent,
}

struct RewriteHandle {
    target: &'static str,
    replacement: InvalidHandle,
    saved: Mutex<Option<Resource>>,
}

impl Middleware for RewriteHandle {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() != self.target {
            return next.run(call).await;
        }
        let Val::Resource(current) = &call.args[0] else {
            panic!("socket call did not receive a resource")
        };
        let replacement = match self.replacement {
            InvalidHandle::Foreign => {
                let foreign = {
                    let mut saved = self.saved.lock().unwrap();
                    if let Some(resource) = saved.as_ref() {
                        Some(resource.clone())
                    } else {
                        *saved = Some(current.clone());
                        None
                    }
                };
                let Some(resource) = foreign else {
                    return next.run(call).await;
                };
                resource
            }
            InvalidHandle::Mistyped => Resource::__borrowed_for_invocation(
                "wasi:io/poll@0.2.12",
                "pollable",
                current.id(),
                call.invocation_id(),
            ),
            InvalidHandle::Unscoped => {
                Resource::borrowed(current.interface(), current.name(), current.id())
            }
            InvalidHandle::Nonexistent => Resource::__borrowed_for_invocation(
                current.interface(),
                current.name(),
                u32::MAX,
                call.invocation_id(),
            ),
        };
        call.args[0] = Val::Resource(replacement);
        next.run(call).await
    }
}

fn assert_invalid_handle(export: &str, target: &'static str, replacement: InvalidHandle) {
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(RewriteHandle {
            target,
            replacement,
            saved: Mutex::new(None),
        })
        .build()
        .unwrap();
    app.configure("handles", WasiSettings::new().sockets(true))
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("handles")))
        .unwrap();
    if matches!(replacement, InvalidHandle::Foreign) {
        runtime
            .block_on(app.call("handles", EXPORT, export, Vec::new()))
            .unwrap_or_else(|error| panic!("{export} setup failed: {error}"));
    }
    let result = runtime
        .block_on(app.call("handles", EXPORT, export, Vec::new()))
        .unwrap_or_else(|error| panic!("{export} replacement failed: {error}"));
    assert_eq!(result, [Val::Bool(true)]);
}

#[test]
fn invalid_socket_handles_are_refused_as_access_errors() {
    let handles = [
        ("socket-handle", "[method]tcp-socket.set-hop-limit"),
        ("network-handle", "resolve-addresses"),
        (
            "resolver-handle",
            "[method]resolve-address-stream.resolve-next-address",
        ),
        (
            "incoming-datagram-handle",
            "[method]incoming-datagram-stream.receive",
        ),
        (
            "outgoing-datagram-handle",
            "[method]outgoing-datagram-stream.check-send",
        ),
    ];
    for (export, target) in handles {
        for replacement in [
            InvalidHandle::Foreign,
            InvalidHandle::Mistyped,
            InvalidHandle::Unscoped,
            InvalidHandle::Nonexistent,
        ] {
            assert_invalid_handle(export, target, replacement);
        }
    }
}
