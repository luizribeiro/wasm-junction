//! WASI Preview 3 outgoing HTTP behavior.

#![cfg(all(feature = "wasi-http", feature = "wasi-p3"))]

mod support;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use support::{HttpServer, TcpProbeServer};
use wasm_junction::{
    App, Call, CallError, Component, LoadError, Middleware, Next, Resource, Val, Vals, WasiSettings,
};
use wasm_junction_wasmtime::WASI_HTTP_INTERFACES;
use wasm_junction_wasmtime::WasmtimeEngine;

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-http-test.wasm"));
const EXPORT: &str = "test:wasi-http/probe@0.1.0";

fn block_on<F: Future>(future: F) -> F::Output {
    runtime().block_on(future)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn connection_closes(server: &HttpServer) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !server.connection_closed() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
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

fn request_with_options(
    app: &App,
    authority: String,
    secure: bool,
    timeout: Duration,
) -> Result<Vals, tokio::time::error::Elapsed> {
    block_on(async {
        tokio::time::timeout(
            Duration::from_secs(5),
            app.call(
                "http",
                EXPORT,
                "request-with-options",
                vec![
                    Val::from(authority),
                    Val::from("/"),
                    Val::from(secure),
                    Val::Option(Some(Box::new(Val::from(
                        u64::try_from(timeout.as_nanos()).unwrap(),
                    )))),
                ],
            ),
        )
        .await
    })
    .map(|result| result.unwrap())
}

fn empty_request(
    app: &App,
    authority: String,
    timeout: Option<Duration>,
) -> Result<Vals, tokio::time::error::Elapsed> {
    block_on(async {
        tokio::time::timeout(
            Duration::from_secs(5),
            app.call(
                "http",
                EXPORT,
                "empty-request",
                vec![
                    Val::from(authority),
                    Val::from("/empty"),
                    timeout.map_or(Val::Option(None), |timeout| {
                        Val::Option(Some(Box::new(Val::from(
                            u64::try_from(timeout.as_nanos()).unwrap(),
                        ))))
                    }),
                ],
            ),
        )
        .await
    })
    .map(|result| result.unwrap())
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

#[test]
fn empty_get_completes() {
    let server = HttpServer::start(b"empty-body-response");
    let app = load(SendPolicy { deny: false }, true);

    let result = empty_request(&app, server.authority(), None).expect("empty GET did not complete");

    assert_eq!(result, [Val::from("200:empty-body-response")]);
    assert!(server.finish().starts_with("GET /empty HTTP/1.1\r\n"));
}

#[test]
fn empty_get_observes_response_body_timeout() {
    let server = HttpServer::stall_after_headers();
    let app = load(SendPolicy { deny: false }, true);

    let result = empty_request(&app, server.authority(), Some(Duration::from_millis(500)))
        .expect("empty GET hung before observing its response-body timeout");

    assert!(matches!(
        &result[..],
        [Val::String(message)] if message.contains("ConnectionReadTimeout")
    ));
    assert!(server.finish().starts_with("GET /empty HTTP/1.1\r\n"));
}

#[test]
fn early_response_does_not_silently_truncate_request_body() {
    const BODY_SIZE: usize = 512 * 1024;

    let server = HttpServer::respond_before_request_body();
    let app = load(SendPolicy { deny: false }, true);

    let result = block_on(app.call(
        "http",
        EXPORT,
        "large-request",
        vec![Val::from(server.authority()), Val::from("/large")],
    ))
    .unwrap();
    let request = server.finish();
    let body = request.split_once("\r\n\r\n").unwrap().1.as_bytes();

    assert!(
        result == [Val::from("sent")] && body == vec![b'x'; BODY_SIZE]
            || matches!(&result[..], [Val::String(message)] if message.contains("ConnectionTerminated"))
    );
}

#[test]
fn dropping_response_closes_its_connection() {
    let server = HttpServer::stall_after_headers();
    let app = load(SendPolicy { deny: false }, true);
    let runtime = runtime();

    let result = runtime
        .block_on(app.call(
            "http",
            EXPORT,
            "drop-response",
            vec![Val::from(server.authority()), Val::from("/drop")],
        ))
        .unwrap();

    assert_eq!(result, [Val::from("dropped")]);
    assert!(runtime.block_on(connection_closes(&server)));
    assert!(server.finish().starts_with("GET /drop HTTP/1.1\r\n"));
}

#[test]
fn dropping_store_closes_an_in_flight_response() {
    let server = HttpServer::stall_after_headers();
    let app = load(SendPolicy { deny: false }, true);
    let runtime = runtime();

    let result = runtime
        .block_on(app.call(
            "http",
            EXPORT,
            "leak-response",
            vec![Val::from(server.authority()), Val::from("/leak")],
        ))
        .unwrap();

    assert_eq!(result, [Val::from("leaked")]);
    assert!(runtime.block_on(connection_closes(&server)));
    assert!(server.finish().starts_with("GET /leak HTTP/1.1\r\n"));
}

#[test]
fn cancelling_call_closes_its_connection() {
    let server = HttpServer::stall_after_headers();
    let app = load(SendPolicy { deny: false }, true);

    let runtime = runtime();
    runtime.block_on(async {
        let mut call = Box::pin(app.call(
            "http",
            EXPORT,
            "hold-response",
            vec![Val::from(server.authority()), Val::from("/hold")],
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !server.has_sent_headers() {
                tokio::select! {
                    result = &mut call => panic!("call completed while waiting for headers: {result:?}"),
                    () = tokio::time::sleep(Duration::from_millis(10)) => {}
                }
            }
            tokio::select! {
                result = &mut call => panic!("call completed while holding the response: {result:?}"),
                () = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        })
        .await
        .expect("guest did not receive the response headers");
        drop(call);
        assert!(connection_closes(&server).await);
    });

    assert!(server.finish().starts_with("GET /hold HTTP/1.1\r\n"));
}

#[test]
fn cancelling_call_during_tls_closes_its_connection() {
    let server = TcpProbeServer::hold_after_prefix();
    let app = load(SendPolicy { deny: false }, true);
    let runtime = runtime();

    runtime.block_on(async {
        let mut call = Box::pin(app.call(
            "http",
            EXPORT,
            "request-with-options",
            vec![
                Val::from(server.authority()),
                Val::from("/"),
                Val::from(true),
                Val::Option(Some(Box::new(Val::from(60_000_000_000_u64)))),
            ],
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !server.has_received_prefix() {
                tokio::select! {
                    result = &mut call => panic!("call completed during TLS setup: {result:?}"),
                    () = tokio::time::sleep(Duration::from_millis(10)) => {}
                }
            }
        })
        .await
        .expect("TLS handshake did not reach the peer");
        drop(call);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !server.connection_closed() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TLS connection remained open after cancellation");
    });

    assert_eq!(&server.finish()[..2], &[0x16, 0x03]);
}

#[test]
fn https_send_reaches_the_wire_as_tls() {
    // This is scheme-preservation coverage, not a regression test for the stall.
    let server = TcpProbeServer::close_after_prefix();
    let app = load(SendPolicy { deny: false }, true);

    let result =
        request_with_options(&app, server.authority(), true, Duration::from_millis(500)).unwrap();

    assert!(matches!(&result[..], [Val::String(message)] if message.starts_with("error: ")));
    assert_eq!(&server.finish()[..3], &[0x16, 0x03, 0x01]);
}

#[test]
fn https_handshake_obeys_request_timeouts() {
    let server = TcpProbeServer::hold_after_prefix();
    let app = load(SendPolicy { deny: false }, true);
    let started = Instant::now();

    let result = request_with_options(&app, server.authority(), true, Duration::from_millis(500))
        .expect("HTTPS request ignored its timeout");

    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(matches!(&result[..], [Val::String(message)] if message.starts_with("error: ")));
    assert_eq!(&server.finish()[..2], &[0x16, 0x03]);
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

struct RecordHttp(Arc<Mutex<BTreeSet<(String, String)>>>);

impl Middleware for RecordHttp {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.starts_with("wasi:http/") {
            self.0
                .lock()
                .unwrap()
                .insert((call.interface.to_string(), call.function.to_string()));
        }
        next.run(call).await
    }
}

#[test]
fn every_function_in_the_resolved_http_wit_has_a_gate() {
    let seen = Arc::new(Mutex::new(BTreeSet::new()));
    let app = load(RecordHttp(seen.clone()), true);
    block_on(app.call("http", EXPORT, "coverage", Vec::new())).unwrap();
    let server = HttpServer::start(b"coverage");
    block_on(app.call(
        "http",
        EXPORT,
        "request",
        vec![Val::from(server.authority()), Val::from("/coverage")],
    ))
    .unwrap();
    server.finish();

    assert_eq!(*seen.lock().unwrap(), http_wit_functions());
}

fn http_wit_functions() -> BTreeSet<(String, String)> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success(), "cargo metadata failed");
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let manifests: Vec<_> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|package| package["name"] == "wasmtime-wasi-http")
        .map(|package| package["manifest_path"].as_str().unwrap())
        .collect();
    assert_eq!(manifests.len(), 1, "expected one resolved HTTP package");
    let wit = Path::new(manifests[0]).parent().unwrap().join("src/p3/wit");
    let mut resolve = wit_parser::Resolve::default();
    resolve.push_dir(wit).unwrap();
    let mut functions = BTreeSet::new();
    for (_, package) in &resolve.packages {
        for (name, interface) in &package.interfaces {
            let interface_name = package.name.interface_id(name);
            if !WASI_HTTP_INTERFACES.contains(&interface_name.as_str()) {
                continue;
            }
            let definition = &resolve.interfaces[*interface];
            for function in definition.functions.keys() {
                functions.insert((interface_name.clone(), function.clone()));
            }
            for (name, type_id) in &definition.types {
                if matches!(
                    resolve.types[*type_id].kind,
                    wit_parser::TypeDefKind::Resource
                ) {
                    functions.insert((interface_name.clone(), format!("[drop]{name}")));
                }
            }
        }
    }
    functions
}
