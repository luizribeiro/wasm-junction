//! WASI Preview 3 outgoing HTTP behavior.

#![cfg(feature = "wasi-http")]

mod support;

use std::sync::{Arc, Mutex};

use support::HttpServer;
use wasm_junction::{App, Call, CallError, Component, Middleware, Next, Val, Vals, WasiSettings};
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
