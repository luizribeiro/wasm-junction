//! WASI Preview 2 outgoing HTTP behavior.

#![cfg(feature = "wasi-http")]

mod support;

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use support::HttpServer;
use wasm_junction::{
    App, Call, CallError, ChannelDirection, Component, Event, Middleware, Next, Resource, Val,
    Vals, WasiSettings,
};
use wasm_junction_wasmtime::WasmtimeEngine;

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-http-p2-test.wasm"));
const EXPORT: &str = "test:wasi-http-p2/probe@0.1.0";
#[cfg(feature = "wasi-p3")]
const P3_COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-http-test.wasm"));
#[cfg(feature = "wasi-p3")]
const P3_EXPORT: &str = "test:wasi-http/probe@0.1.0";

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

type BodyContexts = Arc<Mutex<Vec<(String, Val)>>>;
type ChannelEvents = Arc<Mutex<Vec<(bool, ChannelDirection)>>>;

struct ObserveBodies {
    contexts: BodyContexts,
    events: ChannelEvents,
}

impl Middleware for ObserveBodies {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if matches!(
            call.function.as_ref(),
            "[method]outgoing-body.write" | "[method]incoming-body.stream"
        ) {
            self.contexts
                .lock()
                .unwrap()
                .push((call.function.to_string(), call.args.last().unwrap().clone()));
        }
        next.run(call).await
    }

    fn event(&self, event: &Event) {
        let observed = match event {
            Event::ChannelOpen { direction, .. } => Some((true, *direction)),
            Event::ChannelClose { direction, .. } => Some((false, *direction)),
            _ => None,
        };
        if let Some(observed) = observed {
            self.events.lock().unwrap().push(observed);
        }
    }
}

#[test]
fn body_streams_carry_request_context_and_channel_lifecycle() {
    let server = HttpServer::start(b"body");
    let contexts = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let app = load(
        ObserveBodies {
            contexts: contexts.clone(),
            events: events.clone(),
        },
        true,
    );
    assert_eq!(
        request(&app, server.authority(), "/channels"),
        [Val::from("200:body")]
    );
    server.finish();

    let contexts = contexts.lock().unwrap();
    assert_eq!(contexts.len(), 2);
    for (_, context) in contexts.iter() {
        let Val::Tuple(values) = context else {
            panic!("body call did not carry request context")
        };
        assert_eq!(values[0], variant("post"));
        assert_eq!(
            values[3],
            Val::Option(Some(Box::new(Val::from("/channels"))))
        );
    }
    let events = events.lock().unwrap();
    for direction in [ChannelDirection::GuestToHost, ChannelDirection::HostToGuest] {
        assert!(events.contains(&(true, direction)));
        assert!(events.contains(&(false, direction)));
    }
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
        if call.interface.as_ref() != "wasi:http/outgoing-handler@0.2.12"
            || call.function.as_ref() != "handle"
        {
            return next.run(call).await;
        }
        let Val::Resource(current) = &call.args[0] else {
            return Err(CallError::trap("request handle had the wrong shape"));
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
fn invalid_preview_2_http_handles_are_refused() {
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
            assert_eq!(
                request(&app, "127.0.0.1:9".into(), "/saved"),
                [Val::from("denied")]
            );
        }
        assert_eq!(
            request(&app, "127.0.0.1:9".into(), "/invalid"),
            [Val::from("denied")]
        );
    }
}

#[cfg(feature = "wasi-p3")]
struct RefuseHttpVersions(Arc<Mutex<std::collections::BTreeSet<String>>>);

#[cfg(feature = "wasi-p3")]
impl Middleware for RefuseHttpVersions {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if (call.interface.as_ref() == "wasi:http/outgoing-handler@0.2.12"
            && call.function.as_ref() == "handle")
            || (call.interface.as_ref() == "wasi:http/client@0.3.0"
                && call.function.as_ref() == "send")
        {
            self.0.lock().unwrap().insert(call.interface.to_string());
            return Err(CallError::refused("origin denied"));
        }
        next.run(call).await
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn one_middleware_refuses_both_http_versions() {
    let seen = Arc::new(Mutex::new(std::collections::BTreeSet::new()));
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .provide(wasm_junction::wasi::http::provider())
        .middleware(RefuseHttpVersions(seen.clone()))
        .build()
        .unwrap();
    for name in ["http-p2", "http-p3"] {
        app.configure(name, WasiSettings::new().network(true))
            .unwrap();
    }
    block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("http-p2"))).unwrap();
    block_on(
        app.load(
            Component::from_bytes(P3_COMPONENT)
                .unwrap()
                .named("http-p3"),
        ),
    )
    .unwrap();

    assert_eq!(
        request(&app, "example.invalid".into(), "/p2"),
        [Val::from("denied")]
    );
    assert_eq!(
        block_on(app.call(
            "http-p3",
            P3_EXPORT,
            "request",
            vec![Val::from("example.invalid"), Val::from("/p3")],
        ))
        .unwrap(),
        [Val::from("denied")]
    );
    assert_eq!(
        *seen.lock().unwrap(),
        std::collections::BTreeSet::from([
            "wasi:http/client@0.3.0".into(),
            "wasi:http/outgoing-handler@0.2.12".into(),
        ])
    );
}

#[test]
fn every_stable_function_in_the_resolved_preview_2_http_wit_has_a_gate() {
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
    let wit = Path::new(manifests[0]).parent().unwrap().join("wit");
    let mut resolve = wit_parser::Resolve::default();
    resolve.push_dir(wit).unwrap();

    let gates = concat!(
        include_str!("../src/wasi/gates/http_p2.rs"),
        include_str!("../src/wasi/gates/http_p2/bodies.rs"),
        include_str!("../src/wasi/gates/http_p2/outgoing.rs"),
        include_str!("../src/wasi/gates/http_p2/outgoing_responses.rs"),
        include_str!("../src/wasi/gates/http_p2/requests.rs"),
        include_str!("../src/wasi/gates/http_p2/responses.rs"),
    );
    let mut missing = Vec::new();
    for (_, package) in &resolve.packages {
        for (name, interface) in &package.interfaces {
            let interface_name = package.name.interface_id(name);
            if !matches!(
                interface_name.as_str(),
                "wasi:http/types@0.2.12" | "wasi:http/outgoing-handler@0.2.12"
            ) {
                continue;
            }
            let definition = &resolve.interfaces[*interface];
            for function in definition.functions.keys() {
                if !gates.contains(&format!("\"{function}\"")) {
                    missing.push((interface_name.clone(), function.clone()));
                }
            }
            for (resource, type_id) in &definition.types {
                if matches!(
                    resolve.types[*type_id].kind,
                    wit_parser::TypeDefKind::Resource
                ) && !gates.contains(&format!("\"{resource}\""))
                {
                    missing.push((interface_name.clone(), format!("[drop]{resource}")));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "HTTP functions without gates: {missing:?}"
    );
}
