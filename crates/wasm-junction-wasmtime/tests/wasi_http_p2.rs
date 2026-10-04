//! WASI Preview 2 outgoing HTTP behavior.

#![cfg(feature = "wasi-http")]

mod support;

use std::sync::{Arc, Mutex};

use support::HttpServer;
use wasm_junction::{App, Call, CallError, Component, Middleware, Next, Val, Vals, WasiSettings};
use wasm_junction_wasmtime::WasmtimeEngine;

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-http-p2-test.wasm"));
const EXPORT: &str = "test:wasi-http-p2/probe@0.1.0";

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
    app.configure("http-p2", WasiSettings::new().network(network))
        .unwrap();
    block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("http-p2"))).unwrap();
    app
}

fn request(app: &App, authority: String, path: &str) -> Vals {
    block_on(app.call(
        "http-p2",
        EXPORT,
        "request",
        vec![Val::from(authority), Val::from(path)],
    ))
    .unwrap()
}

struct Policy {
    deny: bool,
    seen: Arc<Mutex<Option<Vals>>>,
}

impl Middleware for Policy {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() != "wasi:http/outgoing-handler@0.2.12"
            || call.function.as_ref() != "handle"
        {
            return next.run(call).await;
        }
        *self.seen.lock().unwrap() = Some(call.args.clone());
        if self.deny {
            return Err(CallError::refused("origin denied"));
        }
        let Val::List(headers) = &mut call.args[5] else {
            return Err(CallError::trap("request headers had the wrong shape"));
        };
        headers.push(Val::Tuple(vec![
            Val::from("x-secret"),
            Val::from(b"middleware".to_vec()),
        ]));
        next.run(call).await
    }
}

fn variant(case: &str) -> Val {
    Val::Variant {
        case: case.into(),
        value: None,
    }
}

#[test]
fn request_context_is_visible_and_header_rewrites_reach_the_wire() {
    let server = HttpServer::start(b"server-body");
    let authority = server.authority();
    let seen = Arc::new(Mutex::new(None));
    let app = load(
        Policy {
            deny: false,
            seen: seen.clone(),
        },
        true,
    );

    assert_eq!(
        request(&app, authority.clone(), "/notes?id=7"),
        [Val::from("200:server-body")]
    );
    let wire = server.finish().to_ascii_lowercase();
    assert!(wire.starts_with("post /notes?id=7 http/1.1\r\n"));
    assert!(wire.contains("x-secret: middleware\r\n"));
    assert!(wire.contains("guest-body"));
    let context = seen.lock().unwrap().clone().unwrap();
    assert!(matches!(&context[0], Val::Resource(resource)
        if resource.name() == "outgoing-request"));
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
    assert!(matches!(&context[5], Val::List(headers) if headers.len() == 1));
    assert_eq!(context[6], Val::Option(None));
}

#[test]
fn refusal_and_disabled_network_return_http_request_denied() {
    let denied = load(
        Policy {
            deny: true,
            seen: Arc::new(Mutex::new(None)),
        },
        true,
    );
    assert_eq!(
        request(&denied, "127.0.0.1:9".into(), "/denied"),
        [Val::from("denied")]
    );

    let disabled = load(
        Policy {
            deny: false,
            seen: Arc::new(Mutex::new(None)),
        },
        false,
    );
    assert_eq!(
        request(&disabled, "127.0.0.1:9".into(), "/disabled"),
        [Val::from("denied")]
    );
}
