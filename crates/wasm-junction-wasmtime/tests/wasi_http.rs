//! WASI Preview 3 outgoing HTTP behavior.

#![cfg(feature = "wasi-http")]

mod support;

use std::sync::{Arc, Mutex};

use support::HttpServer;
use wasm_junction::{
    App, Call, CallError, Component, LoadError, Middleware, Next, Resource, Val, Vals, WasiSettings,
};
use wasm_junction_wasmtime::WasmtimeEngine;

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-http-test.wasm"));
const EXPORT: &str = "test:wasi-http/probe@0.1.0";

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn load(middleware: impl Middleware + 'static, network: bool) -> App {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .provide(wasm_junction::wasi::http::provider())
        .middleware(middleware)
        .build()
        .unwrap();
    app.configure("http", WasiSettings::new().network(network))
        .unwrap();
    block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("http"))).unwrap();
    app
}

struct RewriteHeader(Arc<Mutex<Option<Vals>>>);

impl Middleware for RewriteHeader {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == "wasi:http/client@0.3.0" && call.function.as_ref() == "send" {
            *self.0.lock().unwrap() = Some(call.args.clone());
            let Val::List(headers) = &mut call.args[5] else {
                return Err(CallError::trap("send headers had the wrong shape"));
            };
            headers.push(Val::Tuple(vec![
                Val::from("x-secret"),
                Val::from(b"middleware".to_vec()),
            ]));
        }
        next.run(call).await
    }
}

#[test]
fn send_exposes_policy_context_and_applies_header_rewrites() {
    let server = HttpServer::start(b"server-body");
    let authority = server.authority();
    let seen = Arc::new(Mutex::new(None));
    let app = load(RewriteHeader(seen.clone()), true);

    let result = block_on(app.call(
        "http",
        EXPORT,
        "request",
        vec![Val::from(authority.clone()), Val::from("/notes?id=7")],
    ))
    .unwrap();

    assert_eq!(result, [Val::from("200:server-body")]);
    let request = server.finish().to_ascii_lowercase();
    assert!(request.starts_with("post /notes?id=7 http/1.1\r\n"));
    assert!(request.contains("x-secret: middleware\r\n"));
    assert!(request.contains("guest-body"));
    let context = seen.lock().unwrap().clone().unwrap();
    assert!(matches!(&context[0], Val::Resource(resource) if resource.name() == "request"));
    assert_eq!(context[1], variant("post"));
    assert_eq!(context[2], Val::Option(Some(Box::new(variant("HTTP")))));
    assert_eq!(
        context[3],
        Val::Option(Some(Box::new(Val::from(authority))))
    );
    assert_eq!(
        context[4],
        Val::Option(Some(Box::new(Val::from("/notes?id=7"))))
    );
    assert_eq!(
        context[5],
        Val::List(vec![Val::Tuple(vec![
            Val::from("x-client"),
            Val::from(b"visible".to_vec()),
        ])])
    );
}

fn variant(case: &str) -> Val {
    Val::Variant {
        case: case.into(),
        value: None,
    }
}

struct SendPolicy {
    deny: bool,
}

impl Middleware for SendPolicy {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if self.deny
            && call.interface.as_ref() == "wasi:http/client@0.3.0"
            && call.function.as_ref() == "send"
        {
            return Err(CallError::refused("origin denied"));
        }
        next.run(call).await
    }
}

fn request(app: &App) -> Vals {
    block_on(app.call(
        "http",
        EXPORT,
        "request",
        vec![Val::from("127.0.0.1:9"), Val::from("/denied")],
    ))
    .unwrap()
}

#[test]
fn middleware_refusal_becomes_the_http_denied_error() {
    let app = load(SendPolicy { deny: true }, true);
    assert_eq!(request(&app), [Val::from("denied")]);
}

#[test]
fn disabled_network_becomes_the_http_denied_error() {
    let app = load(SendPolicy { deny: false }, false);
    assert_eq!(request(&app), [Val::from("denied")]);
}

#[derive(Clone, Copy)]
enum HandleFault {
    Foreign,
    Mistyped,
    Unscoped,
}

struct CorruptHandle {
    fault: HandleFault,
    previous: Mutex<Option<Resource>>,
}

impl Middleware for CorruptHandle {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() != "wasi:http/client@0.3.0" || call.function.as_ref() != "send" {
            return next.run(call).await;
        }
        let Val::Resource(current) = &call.args[0] else {
            return Err(CallError::trap("send request had the wrong shape"));
        };
        let replacement = match self.fault {
            HandleFault::Foreign => {
                let mut previous = self.previous.lock().unwrap();
                let Some(resource) = previous.clone() else {
                    *previous = Some(current.clone());
                    return Err(CallError::refused("saved for another invocation"));
                };
                resource
            }
            HandleFault::Mistyped => Resource::owned(current.interface(), "fields", current.id()),
            HandleFault::Unscoped => {
                Resource::owned(current.interface(), current.name(), current.id())
            }
        };
        call.args[0] = Val::Resource(replacement);
        next.run(call).await
    }
}

#[test]
fn invalid_http_handles_are_refused() {
    for fault in [
        HandleFault::Foreign,
        HandleFault::Mistyped,
        HandleFault::Unscoped,
    ] {
        let app = load(
            CorruptHandle {
                fault,
                previous: Mutex::new(None),
            },
            true,
        );
        if matches!(fault, HandleFault::Foreign) {
            assert_eq!(request(&app), [Val::from("denied")]);
        }
        assert_eq!(request(&app), [Val::from("denied")]);
    }
}

struct ReuseTrailerFuture(Mutex<Option<Val>>);

impl Middleware for ReuseTrailerFuture {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() != "wasi:http/types@0.3.0"
            || call.function.as_ref() != "[static]request.new"
        {
            return next.run(call).await;
        }
        {
            let mut previous = self.0.lock().unwrap();
            if let Some(stale) = previous.take() {
                call.args[2] = stale;
            } else {
                *previous = Some(call.args[2].clone());
            }
        }
        next.run(call).await
    }
}

#[test]
fn trailer_future_cannot_be_reused_after_its_invocation() {
    let app = load(ReuseTrailerFuture(Mutex::new(None)), false);
    assert_eq!(request(&app), [Val::from("denied")]);

    let error = block_on(app.call(
        "http",
        EXPORT,
        "request",
        vec![Val::from("127.0.0.1:9"), Val::from("/stale")],
    ))
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not belong to this invocation"),
        "{error:#}"
    );
}

#[test]
fn http_imports_are_missing_without_the_http_provider() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    let error =
        block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("http"))).unwrap_err();
    let LoadError::MissingImports(missing) = error else {
        panic!("expected missing HTTP imports")
    };
    assert_eq!(
        missing.interfaces(),
        ["wasi:http/client@0.3.0", "wasi:http/types@0.3.0"]
    );
}

#[test]
fn every_http_type_shape_completes() {
    let app = load(SendPolicy { deny: false }, false);
    let result = block_on(app.call("http", EXPORT, "coverage", Vec::new())).unwrap();
    assert_eq!(result, [Val::from("ok")]);
}
