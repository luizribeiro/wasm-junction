//! Socket middleware policy and context behavior.

#![cfg(feature = "wasi")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use wasm_junction::{App, Call, CallError, Component, Middleware, Next, Val, Vals, WasiSettings};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const EXPORT: &str = "test:wasi/environment@0.1.0";

fn address(port: u16) -> Val {
    Val::Variant {
        case: "ipv4".to_owned(),
        value: Some(Box::new(Val::Record(vec![
            ("port".to_owned(), Val::U16(port)),
            (
                "address".to_owned(),
                Val::Tuple(vec![Val::U8(127), Val::U8(0), Val::U8(0), Val::U8(1)]),
            ),
        ]))),
    }
}

struct SocketPolicy {
    blocked_port: u16,
    calls: Arc<Mutex<Vec<(String, Vals)>>>,
}

impl Middleware for SocketPolicy {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let relevant = call.interface.as_ref() == "wasi:sockets/tcp@0.2.12"
            && call.function.as_ref() == "[method]tcp-socket.start-connect"
            || call.interface.as_ref() == "wasi:sockets/ip-name-lookup@0.2.12"
                && call.function.as_ref() == "resolve-addresses"
            || call.interface.as_ref() == "wasi:io/streams@0.2.12";
        if relevant {
            self.calls
                .lock()
                .unwrap()
                .push((call.function.to_string(), call.args.clone()));
        }
        if call.function.as_ref() == "[method]tcp-socket.start-connect"
            && call.args.get(2) == Some(&address(self.blocked_port))
        {
            Err(CallError::refused("destination denied"))
        } else {
            next.run(call).await
        }
    }
}

#[test]
fn middleware_can_filter_destinations_and_observe_stream_context() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = [0; 3];
        stream.read_exact(&mut bytes).unwrap();
        stream.write_all(&bytes).unwrap();
    });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(SocketPolicy {
            blocked_port: 1,
            calls: calls.clone(),
        })
        .build()
        .unwrap();
    app.configure("socket-policy", WasiSettings::new().sockets(true))
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(
            app.load(
                Component::from_bytes(COMPONENT)
                    .unwrap()
                    .named("socket-policy"),
            ),
        )
        .unwrap();

    let denied = runtime
        .block_on(app.call("socket-policy", EXPORT, "tcp-echo", vec![Val::U16(1)]))
        .unwrap();
    assert_eq!(
        denied,
        [Val::Result(Err(Some(Box::new(Val::from(
            "PermissionDenied"
        )))))]
    );
    let allowed = runtime
        .block_on(app.call("socket-policy", EXPORT, "tcp-echo", vec![Val::U16(port)]))
        .unwrap();
    assert_eq!(allowed, [Val::Result(Ok(Some(Box::new(Val::from("tcp")))))]);
    peer.join().unwrap();
    let dns = runtime
        .block_on(app.call(
            "socket-policy",
            EXPORT,
            "dns-probe",
            vec![Val::from("localhost")],
        ))
        .unwrap();
    assert_eq!(dns, [Val::from("started")]);

    let calls = calls.lock().unwrap();
    assert!(calls.iter().any(|(name, args)| {
        name == "resolve-addresses" && args.get(1) == Some(&Val::from("localhost"))
    }));
    assert!(
        calls
            .iter()
            .any(|(name, args)| { name.contains("read") && args.last() == Some(&address(port)) })
    );
    assert!(
        calls
            .iter()
            .any(|(name, args)| { name.contains("write") && args.last() == Some(&address(port)) })
    );
}
