//! Differential coverage against wasmtime-wasi's plain linker.

#![cfg(feature = "wasi")]

#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
mod support;

use std::path::PathBuf;
use std::{io, net, thread};

use wasm_junction::{Access, App, Component as JunctionComponent, Val, WasiSettings};
use wasmtime::component::{Component, Linker, ResourceTable, Val as WasmtimeVal};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const EXPORT: &str = "test:wasi/environment@0.1.0";
#[cfg(feature = "wasi-p3")]
const P3_COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-p3-test.wasm"));
#[cfg(feature = "wasi-p3")]
const P3_EXPORT: &str = "test:wasi-p3/probe@0.1.0";
#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
const HTTP_COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-http-test.wasm"));
#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
const HTTP_EXPORT: &str = "test:wasi-http/probe@0.1.0";

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("wasm-junction-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct State {
    context: WasiCtx,
    table: ResourceTable,
    #[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
    http: WasiHttpCtx,
}

impl State {
    fn configured() -> Self {
        let mut builder = WasiCtxBuilder::new();
        builder.env("GREETING", "hello").arg("alpha");
        Self {
            context: builder.build(),
            table: ResourceTable::new(),
            #[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
            http: WasiHttpCtx::new(),
        }
    }

    fn with_preopen(path: &std::path::Path) -> Self {
        let mut builder = WasiCtxBuilder::new();
        builder
            .preopened_dir(path, "/data", FsPerms::ReadWrite)
            .unwrap();
        Self {
            context: builder.build(),
            table: ResourceTable::new(),
            #[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
            http: WasiHttpCtx::new(),
        }
    }

    fn with_sockets() -> Self {
        let mut builder = WasiCtxBuilder::new();
        builder
            .inherit_network()
            .allow_ip_name_lookup(true)
            .allow_tcp(true)
            .allow_udp(true);
        Self {
            context: builder.build(),
            table: ResourceTable::new(),
            #[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
            http: WasiHttpCtx::new(),
        }
    }
}

#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
impl WasiHttpView for State {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: Default::default(),
        }
    }
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.context,
            table: &mut self.table,
        }
    }
}

async fn plain() -> String {
    let mut config = Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true);
    let engine = Engine::new(&config).unwrap();
    let component = Component::new(&engine, COMPONENT).unwrap();
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
    let pre = linker.instantiate_pre(&component).unwrap();
    let mut store = Store::new(&engine, State::configured());
    let instance = pre.instantiate_async(&mut store).await.unwrap();
    let interface = instance.get_export_index(&mut store, None, EXPORT).unwrap();
    let function = instance
        .get_export_index(&mut store, Some(&interface), "differential")
        .unwrap();
    let function = instance.get_func(&mut store, function).unwrap();
    let mut results = vec![WasmtimeVal::Bool(false)];
    store
        .run_concurrent(async |accessor| {
            function.call_concurrent(accessor, &[], &mut results).await
        })
        .await
        .unwrap()
        .unwrap();
    let [WasmtimeVal::String(result)] = results.as_slice() else {
        panic!("plain WASI returned the wrong shape")
    };
    result.clone()
}

async fn plain_filesystem(path: &std::path::Path) -> String {
    let mut config = Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true);
    let engine = Engine::new(&config).unwrap();
    let component = Component::new(&engine, COMPONENT).unwrap();
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
    let pre = linker.instantiate_pre(&component).unwrap();
    let mut store = Store::new(&engine, State::with_preopen(path));
    let instance = pre.instantiate_async(&mut store).await.unwrap();
    let interface = instance.get_export_index(&mut store, None, EXPORT).unwrap();
    let function = instance
        .get_export_index(&mut store, Some(&interface), "filesystem-differential")
        .unwrap();
    let function = instance.get_func(&mut store, function).unwrap();
    let mut results = vec![WasmtimeVal::String(String::new())];
    store
        .run_concurrent(async |accessor| {
            function.call_concurrent(accessor, &[], &mut results).await
        })
        .await
        .unwrap()
        .unwrap();
    let [WasmtimeVal::String(result)] = results.as_slice() else {
        panic!("plain filesystem returned the wrong shape")
    };
    result.clone()
}

async fn plain_sockets(tcp_port: u16, udp_port: u16) -> String {
    let mut config = Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true);
    let engine = Engine::new(&config).unwrap();
    let component = Component::new(&engine, COMPONENT).unwrap();
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
    let pre = linker.instantiate_pre(&component).unwrap();
    let mut store = Store::new(&engine, State::with_sockets());
    let instance = pre.instantiate_async(&mut store).await.unwrap();
    let interface = instance.get_export_index(&mut store, None, EXPORT).unwrap();
    let function = instance
        .get_export_index(&mut store, Some(&interface), "socket-differential")
        .unwrap();
    let function = instance.get_func(&mut store, function).unwrap();
    let params = [WasmtimeVal::U16(tcp_port), WasmtimeVal::U16(udp_port)];
    let mut results = vec![WasmtimeVal::Result(Ok(Some(Box::new(
        WasmtimeVal::String(String::new()),
    ))))];
    store
        .run_concurrent(async |accessor| {
            function
                .call_concurrent(accessor, &params, &mut results)
                .await
        })
        .await
        .unwrap()
        .unwrap();
    let [WasmtimeVal::Result(Ok(Some(result)))] = results.as_slice() else {
        panic!("plain sockets returned the wrong shape")
    };
    let WasmtimeVal::String(result) = result.as_ref() else {
        panic!("plain sockets returned a non-string result")
    };
    result.clone()
}

fn echo_peers() -> (u16, u16, thread::JoinHandle<()>, thread::JoinHandle<()>) {
    use io::{Read, Write};

    let tcp = net::TcpListener::bind("127.0.0.1:0").unwrap();
    let tcp_port = tcp.local_addr().unwrap().port();
    let tcp_peer = thread::spawn(move || {
        let (mut stream, _) = tcp.accept().unwrap();
        let mut bytes = [0; 3];
        stream.read_exact(&mut bytes).unwrap();
        stream.write_all(&bytes).unwrap();
    });
    let udp = net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_port = udp.local_addr().unwrap().port();
    let udp_peer = thread::spawn(move || {
        let mut bytes = [0; 3];
        let (length, peer) = udp.recv_from(&mut bytes).unwrap();
        udp.send_to(&bytes[..length], peer).unwrap();
    });
    (tcp_port, udp_port, tcp_peer, udp_peer)
}

#[cfg(feature = "wasi-p3")]
async fn plain_p3() -> String {
    let mut config = Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true);
    let engine = Engine::new(&config).unwrap();
    let component = Component::new(&engine, P3_COMPONENT).unwrap();
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p3::add_to_linker(&mut linker).unwrap();
    let pre = linker.instantiate_pre(&component).unwrap();
    let mut store = Store::new(&engine, State::configured());
    let instance = pre.instantiate_async(&mut store).await.unwrap();
    let interface = instance
        .get_export_index(&mut store, None, P3_EXPORT)
        .unwrap();
    let function = instance
        .get_export_index(&mut store, Some(&interface), "coverage")
        .unwrap();
    let function = instance.get_func(&mut store, function).unwrap();
    let mut results = vec![WasmtimeVal::Bool(false)];
    store
        .run_concurrent(async |accessor| {
            function.call_concurrent(accessor, &[], &mut results).await
        })
        .await
        .unwrap()
        .unwrap();
    let [WasmtimeVal::String(result)] = results.as_slice() else {
        panic!("plain Preview 3 WASI returned the wrong shape")
    };
    result.clone()
}

#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
async fn plain_http(authority: String) -> String {
    let mut config = Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true);
    let engine = Engine::new(&config).unwrap();
    let component = Component::new(&engine, HTTP_COMPONENT).unwrap();
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p3::add_to_linker(&mut linker).unwrap();
    wasmtime_wasi_http::p3::add_to_linker(&mut linker).unwrap();
    let pre = linker.instantiate_pre(&component).unwrap();
    let mut store = Store::new(&engine, State::configured());
    let instance = pre.instantiate_async(&mut store).await.unwrap();
    let interface = instance
        .get_export_index(&mut store, None, HTTP_EXPORT)
        .unwrap();
    let function = instance
        .get_export_index(&mut store, Some(&interface), "request")
        .unwrap();
    let function = instance.get_func(&mut store, function).unwrap();
    let mut results = vec![WasmtimeVal::String(String::new())];
    let params = [
        WasmtimeVal::String(authority),
        WasmtimeVal::String("/differential".into()),
    ];
    store
        .run_concurrent(async |accessor| {
            function
                .call_concurrent(accessor, &params, &mut results)
                .await
        })
        .await
        .unwrap()
        .unwrap();
    let [WasmtimeVal::String(result)] = results.as_slice() else {
        panic!("plain HTTP returned the wrong shape")
    };
    result.clone()
}

#[test]
fn gated_and_plain_wasi_match_for_standard_interfaces() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    app.configure(
        "gated",
        WasiSettings::new().env("GREETING", "hello").arg("alpha"),
    )
    .unwrap();
    runtime
        .block_on(
            app.load(
                JunctionComponent::from_bytes(COMPONENT)
                    .unwrap()
                    .named("gated"),
            ),
        )
        .unwrap();

    let gated = runtime
        .block_on(app.call("gated", EXPORT, "differential", Vec::new()))
        .unwrap();
    let [Val::String(gated)] = gated.as_slice() else {
        panic!("gated WASI returned the wrong shape")
    };
    assert_eq!(gated, &runtime.block_on(plain()));
}

#[test]
fn gated_and_plain_filesystems_match_on_temporary_directories() {
    let root = TestDirectory::new("differential");
    let gated_dir = root.0.join("gated");
    let plain_dir = root.0.join("plain");
    for directory in [&gated_dir, &plain_dir] {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join("note.txt"), "note").unwrap();
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    app.configure(
        "gated-files",
        WasiSettings::new().preopen(&gated_dir, "/data", Access::ReadWrite),
    )
    .unwrap();
    runtime
        .block_on(
            app.load(
                JunctionComponent::from_bytes(COMPONENT)
                    .unwrap()
                    .named("gated-files"),
            ),
        )
        .unwrap();
    let gated = runtime
        .block_on(app.call("gated-files", EXPORT, "filesystem-differential", Vec::new()))
        .unwrap();
    let plain = runtime.block_on(plain_filesystem(&plain_dir));
    assert_eq!(gated, [Val::from(plain)]);
    assert_eq!(
        std::fs::read_to_string(gated_dir.join("output.txt")).unwrap(),
        "written"
    );
    assert_eq!(
        std::fs::read_to_string(plain_dir.join("output.txt")).unwrap(),
        "written"
    );
}

#[test]
fn gated_and_plain_tcp_and_udp_sockets_match() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let gated_peers = echo_peers();
    let plain_peers = echo_peers();
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    app.configure("gated-sockets", WasiSettings::new().sockets(true))
        .unwrap();
    runtime
        .block_on(
            app.load(
                JunctionComponent::from_bytes(COMPONENT)
                    .unwrap()
                    .named("gated-sockets"),
            ),
        )
        .unwrap();
    let gated = runtime
        .block_on(app.call(
            "gated-sockets",
            EXPORT,
            "socket-differential",
            vec![Val::U16(gated_peers.0), Val::U16(gated_peers.1)],
        ))
        .unwrap();
    let plain = runtime.block_on(plain_sockets(plain_peers.0, plain_peers.1));
    assert_eq!(gated, [Val::Result(Ok(Some(Box::new(Val::from(plain)))))]);
    for peer in [gated_peers.2, gated_peers.3, plain_peers.2, plain_peers.3] {
        peer.join().unwrap();
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn gated_and_plain_wasi_match_for_preview_3_interfaces() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    runtime
        .block_on(
            app.load(
                JunctionComponent::from_bytes(P3_COMPONENT)
                    .unwrap()
                    .named("gated-p3"),
            ),
        )
        .unwrap();

    let gated = runtime
        .block_on(app.call("gated-p3", P3_EXPORT, "coverage", Vec::new()))
        .unwrap();
    let [Val::String(gated)] = gated.as_slice() else {
        panic!("gated Preview 3 WASI returned the wrong shape")
    };
    assert_eq!(gated, &runtime.block_on(plain_p3()));
}

#[test]
#[cfg(all(feature = "wasi-http", feature = "wasi-p3"))]
fn gated_and_plain_outgoing_http_match() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let gated_server = support::HttpServer::start(b"differential");
    let plain_server = support::HttpServer::start(b"differential");
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .provide(wasm_junction::wasi::http::provider())
        .build()
        .unwrap();
    app.configure("gated-http", WasiSettings::new().network(true))
        .unwrap();
    runtime
        .block_on(
            app.load(
                JunctionComponent::from_bytes(HTTP_COMPONENT)
                    .unwrap()
                    .named("gated-http"),
            ),
        )
        .unwrap();
    let gated = runtime
        .block_on(app.call(
            "gated-http",
            HTTP_EXPORT,
            "request",
            vec![
                Val::from(gated_server.authority()),
                Val::from("/differential"),
            ],
        ))
        .unwrap();
    let plain = runtime.block_on(plain_http(plain_server.authority()));
    assert_eq!(gated, [Val::from(plain)]);
    for request in [gated_server.finish(), plain_server.finish()] {
        assert!(request.starts_with("POST /differential HTTP/1.1\r\n"));
        assert!(request.contains("guest-body"));
    }
}
