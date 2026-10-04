//! WASI Preview 2 socket behavior.

#![cfg(feature = "wasi")]

use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::thread::{self, JoinHandle};

use wasm_junction::{App, Component, Val, WasiSettings};

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
