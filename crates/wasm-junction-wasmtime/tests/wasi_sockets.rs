//! WASI Preview 2 socket behavior.

#![cfg(feature = "wasi")]

use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use wasm_junction::{
    App, Call, CallError, ChannelDirection, Component, Event, InvocationId, Middleware, Next, Val,
    Vals, WasiSettings,
};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const EXPORT: &str = "test:wasi/environment@0.1.0";

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn load(settings: WasiSettings) -> App {
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    app.configure("sockets", settings).unwrap();
    block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("sockets"))).unwrap();
    app
}

fn tcp_echo_server() -> (u16, JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = [0; 3];
        stream.read_exact(&mut bytes).unwrap();
        stream.write_all(&bytes).unwrap();
        bytes.to_vec()
    });
    (port, thread)
}

fn udp_echo_server() -> (u16, JoinHandle<Vec<u8>>) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    let thread = thread::spawn(move || {
        let mut bytes = [0; 3];
        let (length, peer) = socket.recv_from(&mut bytes).unwrap();
        socket.send_to(&bytes[..length], peer).unwrap();
        bytes[..length].to_vec()
    });
    (port, thread)
}

#[test]
fn sockets_and_name_lookup_are_denied_by_default() {
    let app = load(WasiSettings::new());
    let result = block_on(app.call("sockets", EXPORT, "sockets-denied", Vec::new())).unwrap();

    assert_eq!(
        result,
        [Val::Tuple(vec![
            Val::from("access-denied"),
            Val::from("access-denied")
        ])]
    );
}

#[test]
fn http_network_access_does_not_enable_raw_sockets() {
    let app = load(WasiSettings::new().network(true));
    let result = block_on(app.call("sockets", EXPORT, "sockets-denied", Vec::new())).unwrap();

    assert_eq!(
        result,
        [Val::Tuple(vec![
            Val::from("access-denied"),
            Val::from("access-denied")
        ])]
    );
}

#[test]
fn enabled_tcp_and_udp_sockets_reach_loopback_peers() {
    let (tcp_port, tcp_thread) = tcp_echo_server();
    let (udp_port, udp_thread) = udp_echo_server();
    let app = load(WasiSettings::new().sockets(true));

    for (function, port, expected) in [("tcp-echo", tcp_port, "tcp"), ("udp-echo", udp_port, "udp")]
    {
        let result = block_on(app.call("sockets", EXPORT, function, vec![Val::U16(port)])).unwrap();
        assert_eq!(
            result,
            [Val::Result(Ok(Some(Box::new(Val::from(expected)))))]
        );
    }
    assert_eq!(tcp_thread.join().unwrap(), b"tcp");
    assert_eq!(udp_thread.join().unwrap(), b"udp");
}

type ChannelEvent = (bool, InvocationId, u64, ChannelDirection);

struct RecordSocketChannels {
    calls: Arc<Mutex<Vec<(String, InvocationId, ChannelDirection)>>>,
    events: Arc<Mutex<Vec<ChannelEvent>>>,
}

impl Middleware for RecordSocketChannels {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let creates_channels = matches!(
            call.function.as_ref(),
            "[method]tcp-socket.finish-connect" | "[method]udp-socket.stream"
        );
        let function = call.function.to_string();
        let invocation = call.invocation_id();
        let result = next.run(call).await?;
        if creates_channels && matches!(result.first(), Some(Val::Result(Ok(_)))) {
            for direction in [ChannelDirection::HostToGuest, ChannelDirection::GuestToHost] {
                self.calls
                    .lock()
                    .unwrap()
                    .push((function.clone(), invocation, direction));
            }
        }
        Ok(result)
    }

    fn event(&self, event: &Event) {
        let observed = match event {
            Event::ChannelOpen {
                invocation,
                stream,
                direction,
            } => Some((true, *invocation, *stream, *direction)),
            Event::ChannelClose {
                invocation,
                stream,
                direction,
            } => Some((false, *invocation, *stream, *direction)),
            _ => None,
        };
        if let Some(observed) = observed {
            self.events.lock().unwrap().push(observed);
        }
    }
}

#[test]
fn tcp_and_udp_streams_open_and_close_invocation_channels() {
    let (tcp_port, tcp_thread) = tcp_echo_server();
    let (udp_port, udp_thread) = udp_echo_server();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(RecordSocketChannels {
            calls: calls.clone(),
            events: events.clone(),
        })
        .build()
        .unwrap();
    app.configure("channels", WasiSettings::new().sockets(true))
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("channels")))
        .unwrap();
    for (function, port) in [("tcp-echo", tcp_port), ("udp-echo", udp_port)] {
        runtime
            .block_on(app.call("channels", EXPORT, function, vec![Val::U16(port)]))
            .unwrap();
    }
    tcp_thread.join().unwrap();
    udp_thread.join().unwrap();

    let calls = calls.lock().unwrap();
    let events = events.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|call| call.0 == "[method]tcp-socket.finish-connect")
    );
    assert!(
        calls
            .iter()
            .any(|call| call.0 == "[method]udp-socket.stream")
    );
    assert_eq!(events.len(), calls.len() * 2);
    for (_, invocation, direction) in calls.iter() {
        let matching = events
            .iter()
            .filter(|event| event.1 == *invocation && event.3 == *direction)
            .collect::<Vec<_>>();
        assert!(matching.iter().any(|event| event.0));
        assert!(matching.iter().any(|event| !event.0));
    }
    for opened in events.iter().filter(|event| event.0) {
        assert!(events.iter().any(|closed| {
            !closed.0 && closed.1 == opened.1 && closed.2 == opened.2 && closed.3 == opened.3
        }));
    }
}
