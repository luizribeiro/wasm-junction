//! WASI Preview 3 feature and gate behavior.

#![cfg(feature = "wasi")]

use std::future::Future;
#[cfg(feature = "wasi-p3")]
use std::io::{Read, Write};
#[cfg(feature = "wasi-p3")]
use std::net::{Shutdown, TcpStream};
#[cfg(feature = "wasi-p3")]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(feature = "wasi-p3")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "wasi-p3")]
use std::{collections::BTreeSet, path::Path, process::Command};

#[cfg(feature = "wasi-p3")]
use tokio::sync::Notify;
use wasm_junction::LoadError;
use wasm_junction::{Access, App, Component};
#[cfg(feature = "wasi-p3")]
use wasm_junction::{
    Call, CallError, CallErrorKind, ChannelDirection, Event, InputStream, Middleware, Next,
    OutputStream, OutputStreamWriter, StreamHandle, Val, Vals,
};
#[cfg(feature = "wasi-p3")]
use wasm_junction_wasmtime::WASI_INTERFACES;
use wasm_junction_wasmtime::WasmtimeEngine;
#[cfg(feature = "wasi-p3")]
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
#[cfg(feature = "wasi-p3")]
use wit_parser::{ManglingAndAbi, Resolve};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-p3-test.wasm"));
#[cfg(feature = "wasi-p3")]
const EXPORT: &str = "test:wasi-p3/probe@0.1.0";
#[cfg(feature = "wasi-p3")]
const P2_COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
#[cfg(feature = "wasi-p3")]
const P2_EXPORT: &str = "test:wasi/environment@0.1.0";

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
#[cfg(not(feature = "wasi-p3"))]
fn p3_imports_are_missing_when_the_feature_is_disabled() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    let component = Component::from_bytes(COMPONENT).unwrap().named("p3");
    let mut expected = component.imports().to_vec();
    expected.sort();

    let LoadError::MissingImports(missing) = block_on(app.load(component)).unwrap_err() else {
        panic!("expected missing p3 imports");
    };
    assert_eq!(missing.interfaces(), expected);
}

#[cfg(feature = "wasi-p3")]
struct WaitBehavior {
    refuse: bool,
    calls: Arc<AtomicUsize>,
}

#[cfg(feature = "wasi-p3")]
impl Middleware for WaitBehavior {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        let wait = call.interface.as_ref() == "wasi:clocks/monotonic-clock@0.3.0"
            && matches!(call.function.as_ref(), "wait-for" | "wait-until");
        if wait {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.refuse && call.function.as_ref() == "wait-for" {
                return Err(CallError::refused("p3 wait denied"));
            }
            call.args = vec![Val::U64(0)];
        }
        next.run(call).await
    }
}

#[cfg(feature = "wasi-p3")]
fn p3_app(middleware: impl Middleware + 'static) -> App {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(middleware)
        .build()
        .unwrap();
    app.configure(
        "p3",
        wasm_junction::WasiSettings::new()
            .env("GREETING", "hello")
            .arg("alpha"),
    )
    .unwrap();
    block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("p3"))).unwrap();
    app
}

#[cfg(feature = "wasi-p3")]
struct TestDirectory(std::path::PathBuf);

#[cfg(feature = "wasi-p3")]
impl TestDirectory {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("wasm-junction-p3-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

#[cfg(feature = "wasi-p3")]
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(feature = "wasi-p3")]
struct Pass;

#[cfg(feature = "wasi-p3")]
impl Middleware for Pass {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        next.run(call).await
    }
}

#[cfg(feature = "wasi-p3")]
fn filesystem_app(middleware: impl Middleware + 'static, directory: &TestDirectory) -> App {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(middleware)
        .build()
        .unwrap();
    app.configure(
        "p3",
        wasm_junction::WasiSettings::new().preopen(&directory.0, "/data", Access::ReadWrite),
    )
    .unwrap();
    block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("p3"))).unwrap();
    app
}

#[cfg(feature = "wasi-p3")]
fn sockets_app(middleware: impl Middleware + 'static) -> App {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .middleware(middleware)
        .build()
        .unwrap();
    app.configure("p3", wasm_junction::WasiSettings::new().sockets(true))
        .unwrap();
    block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("p3"))).unwrap();
    app
}

#[cfg(feature = "wasi-p3")]
fn socket_port(value: &Val) -> Option<u16> {
    let Val::Variant {
        case,
        value: Some(value),
    } = value
    else {
        return None;
    };
    if case != "ipv4" {
        return None;
    }
    let Val::Record(fields) = value.as_ref() else {
        return None;
    };
    fields.iter().find_map(|(name, value)| {
        (name == "port")
            .then_some(value)
            .and_then(|value| match value {
                Val::U16(port) => Some(*port),
                _ => None,
            })
    })
}

#[cfg(feature = "wasi-p3")]
struct AnnounceListener(Mutex<Option<std::sync::mpsc::Sender<u16>>>);

#[cfg(feature = "wasi-p3")]
impl Middleware for AnnounceListener {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let port = (call.interface.as_ref() == "wasi:sockets/types@0.3.0"
            && call.function.as_ref() == "[method]tcp-socket.listen")
            .then(|| call.args.get(1).and_then(socket_port))
            .flatten();
        if let Some(port) = port
            && let Some(sender) = self.0.lock().unwrap().take()
        {
            sender.send(port).unwrap();
        }
        next.run(call).await
    }
}

#[cfg(feature = "wasi-p3")]
fn tcp_request(port: u16, request: &[u8]) -> Vec<u8> {
    let mut client = tcp_connect(port);
    client.write_all(request).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    response
}

#[cfg(feature = "wasi-p3")]
fn tcp_connect(port: u16) -> TcpStream {
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(client) => return client,
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn tcp_accept_stream_echoes_two_complete_requests() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let clients = std::thread::spawn(move || {
        let port = receiver.recv().unwrap();
        [
            tcp_request(port, b"first request"),
            tcp_request(port, b"second request"),
        ]
    });
    let app = sockets_app(AnnounceListener(Mutex::new(Some(sender))));

    let result = block_on(app.call("p3", EXPORT, "tcp-echo", vec![Val::U8(2)])).unwrap();
    assert_eq!(
        result,
        [Val::Result(Ok(Some(Box::new(Val::List(vec![
            Val::from("first request"),
            Val::from("second request"),
        ])))))]
    );
    assert_eq!(
        clients.join().unwrap(),
        [b"first request".to_vec(), b"second request".to_vec()]
    );
}

#[cfg(feature = "wasi-p3")]
fn accept_stream(values: &mut Vals) -> &mut StreamHandle {
    let [Val::Result(Ok(Some(value)))] = values.as_mut_slice() else {
        panic!("listen returned the wrong shape")
    };
    let Val::Stream(stream) = value.as_mut() else {
        panic!("listen returned no stream")
    };
    stream
}

#[cfg(feature = "wasi-p3")]
struct FilterAccepted {
    sender: Mutex<Option<std::sync::mpsc::Sender<u16>>>,
    accepted: Arc<AtomicUsize>,
    filtered: Arc<Mutex<BTreeSet<u32>>>,
    dropped: Arc<AtomicUsize>,
}

#[cfg(feature = "wasi-p3")]
impl Middleware for FilterAccepted {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let listen = call.interface.as_ref() == "wasi:sockets/types@0.3.0"
            && call.function.as_ref() == "[method]tcp-socket.listen";
        if listen
            && let Some(port) = call.args.get(1).and_then(socket_port)
            && let Some(sender) = self.sender.lock().unwrap().take()
        {
            sender.send(port).unwrap();
        }
        let mut values = next.run(call).await?;
        if listen {
            let accepted = self.accepted.clone();
            let filtered = self.filtered.clone();
            let stream = accept_stream(&mut values);
            *stream = stream.take().filter_items(move |item| {
                let keep = matches!(accepted.fetch_add(1, Ordering::Relaxed), 0 | 4..);
                if !keep && let Val::Resource(resource) = item {
                    filtered.lock().unwrap().insert(resource.id());
                }
                keep
            });
        }
        Ok(values)
    }

    fn event(&self, event: &Event) {
        if matches!(
            event,
            Event::ResourceDrop { interface, resource, id, .. }
                if interface.as_ref() == "wasi:sockets/types@0.3.0"
                    && resource.as_ref() == "tcp-socket"
                    && self.filtered.lock().unwrap().contains(id)
        ) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(feature = "wasi-p3")]
fn discarded_request(port: u16, request: &'static [u8]) -> std::thread::JoinHandle<bool> {
    let mut client = tcp_connect(port);
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    client.write_all(request).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    std::thread::spawn(move || {
        let mut response = Vec::new();
        match client.read_to_end(&mut response) {
            Ok(_) => response.is_empty(),
            Err(error) => error.kind() == std::io::ErrorKind::ConnectionReset,
        }
    })
}

#[cfg(feature = "wasi-p3")]
fn wait_for_accept(accepted: &AtomicUsize, count: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while accepted.load(Ordering::Relaxed) < count {
        assert!(std::time::Instant::now() < deadline, "accept timed out");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn middleware_can_drop_an_accepted_socket() {
    let (port_sender, port_receiver) = std::sync::mpsc::channel();
    let accepted = Arc::new(AtomicUsize::new(0));
    let client_accepted = accepted.clone();
    let clients = std::thread::spawn(move || {
        let port = port_receiver.recv().unwrap();
        let first = tcp_request(port, b"first");
        let mut discarded = Vec::new();
        for (index, request) in [
            b"discarded one".as_slice(),
            b"discarded two",
            b"discarded three",
        ]
        .into_iter()
        .enumerate()
        {
            discarded.push(discarded_request(port, request));
            wait_for_accept(&client_accepted, index + 2);
        }
        let last = tcp_request(port, b"last");
        let discarded = discarded
            .into_iter()
            .map(|client| client.join().unwrap())
            .collect::<Vec<_>>();
        (first, discarded, last)
    });
    let dropped = Arc::new(AtomicUsize::new(0));
    let app = sockets_app(FilterAccepted {
        sender: Mutex::new(Some(port_sender)),
        accepted: accepted.clone(),
        filtered: Arc::new(Mutex::new(BTreeSet::new())),
        dropped: dropped.clone(),
    });

    let result = block_on(app.call("p3", EXPORT, "tcp-echo", vec![Val::U8(2)])).unwrap();
    assert_eq!(
        result,
        [Val::Result(Ok(Some(Box::new(Val::List(vec![
            Val::from("first"),
            Val::from("last"),
        ])))))]
    );
    assert_eq!(
        clients.join().unwrap(),
        (b"first".to_vec(), vec![true; 3], b"last".to_vec())
    );
    assert_eq!(accepted.load(Ordering::Relaxed), 5);
    assert_eq!(dropped.load(Ordering::Relaxed), 3);
}

#[test]
#[cfg(feature = "wasi-p3")]
fn tcp_stream_failures_reach_the_guest() {
    let app = sockets_app(Pass);

    let result = block_on(app.call("p3", EXPORT, "tcp-stream-failures", Vec::new())).unwrap();
    assert_eq!(
        result,
        [Val::List(vec![
            Val::from("ErrorCode::InvalidState"),
            Val::from("ErrorCode::InvalidState")
        ])]
    );
}

#[test]
#[cfg(feature = "wasi-p3")]
fn filesystem_streams_write_append_read_and_list() {
    let directory = TestDirectory::new("streams");
    std::fs::write(directory.0.join("first.txt"), b"first").unwrap();
    std::fs::write(directory.0.join("second.txt"), b"second").unwrap();
    let app = filesystem_app(Pass, &directory);

    let result = block_on(app.call("p3", EXPORT, "filesystem", Vec::new())).unwrap();
    let [Val::String(result)] = result.as_slice() else {
        panic!("filesystem probe returned the wrong shape")
    };
    let expected = std::fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(result, &format!("hello world|{expected}"));
    assert_eq!(
        std::fs::read(directory.0.join("note.txt")).unwrap(),
        b"hello world"
    );
}

#[cfg(feature = "wasi-p3")]
struct HideEntry;

#[cfg(feature = "wasi-p3")]
impl Middleware for HideEntry {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let hide = call.interface.as_ref() == "wasi:filesystem/types@0.3.0"
            && call.function.as_ref() == "[method]descriptor.read-directory";
        let mut values = next.run(call).await?;
        if hide {
            let [Val::Tuple(pair)] = values.as_mut_slice() else {
                return Err(CallError::trap("directory read returned the wrong shape"));
            };
            let Some(Val::Stream(stream)) = pair.first_mut() else {
                return Err(CallError::trap("directory read returned no stream"));
            };
            *stream = stream.take().filter_items(|item| {
                let Val::Record(fields) = item else {
                    return true;
                };
                !fields.iter().any(|(name, value)| {
                    name == "name" && value == &Val::String("hidden.txt".to_owned())
                })
            });
        }
        Ok(values)
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn middleware_filters_directory_entries_before_the_guest_reads_them() {
    let directory = TestDirectory::new("filter");
    std::fs::write(directory.0.join("visible.txt"), b"visible").unwrap();
    std::fs::write(directory.0.join("hidden.txt"), b"hidden").unwrap();
    let app = filesystem_app(HideEntry, &directory);

    let result = block_on(app.call("p3", EXPORT, "list-directory", Vec::new())).unwrap();
    assert_eq!(
        result,
        [Val::Result(Ok(Some(Box::new(Val::List(vec![Val::from(
            "visible.txt"
        )])))))]
    );
}

#[cfg(feature = "wasi-p3")]
struct RefuseDirectoryOnce(AtomicBool);

#[cfg(feature = "wasi-p3")]
impl Middleware for RefuseDirectoryOnce {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() == "[method]descriptor.read-directory"
            && !self.0.swap(true, Ordering::Relaxed)
        {
            Err(CallError::refused("directory listing denied"))
        } else {
            next.run(call).await
        }
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn refused_directory_read_returns_access_and_the_next_call_works() {
    let directory = TestDirectory::new("refusal");
    std::fs::write(directory.0.join("visible.txt"), b"visible").unwrap();
    let app = filesystem_app(RefuseDirectoryOnce(AtomicBool::new(false)), &directory);

    let refused = block_on(app.call("p3", EXPORT, "list-directory", Vec::new())).unwrap();
    assert_eq!(
        refused,
        [Val::Result(Err(Some(Box::new(Val::from(
            "ErrorCode::Access"
        )))))]
    );
    let allowed = block_on(app.call("p3", EXPORT, "list-directory", Vec::new())).unwrap();
    assert_eq!(
        allowed,
        [Val::Result(Ok(Some(Box::new(Val::List(vec![Val::from(
            "visible.txt"
        )])))))]
    );
}

#[test]
#[cfg(all(feature = "wasi-p3", unix))]
fn directory_stream_failure_reaches_the_guest() {
    use std::os::unix::ffi::OsStringExt;

    let directory = TestDirectory::new("failure");
    let invalid = std::ffi::OsString::from_vec(vec![0xff]);
    std::fs::write(directory.0.join(invalid), b"invalid name").unwrap();
    let app = filesystem_app(Pass, &directory);

    let result = block_on(app.call("p3", EXPORT, "list-directory", Vec::new())).unwrap();
    assert_eq!(
        result,
        [Val::Result(Err(Some(Box::new(Val::from(
            "ErrorCode::IllegalByteSequence"
        )))))]
    );
}

#[cfg(feature = "wasi-p3")]
struct DirectoryEvents(ChannelEvents);

#[cfg(feature = "wasi-p3")]
impl Middleware for DirectoryEvents {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        next.run(call).await
    }

    fn event(&self, event: &Event) {
        record_channel_event(&self.0, event);
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn dropping_directory_listing_stops_its_pump_and_preserves_the_next_call() {
    let directory = TestDirectory::new("drop");
    std::fs::write(directory.0.join("visible.txt"), b"visible").unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let app = filesystem_app(DirectoryEvents(events.clone()), &directory);

    let result = block_on(app.call("p3", EXPORT, "drop-directory-listing", Vec::new())).unwrap();
    assert_eq!(result, [Val::List(vec![Val::from("visible.txt")])]);
    let events = events.lock().unwrap();
    assert_eq!(events.iter().filter(|event| event.0).count(), 4);
    assert_eq!(events.iter().filter(|event| !event.0).count(), 4);
}

#[test]
#[cfg(feature = "wasi-p3")]
fn preview_3_environment_and_arguments_match_preview_2() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();
    for component in ["p2", "p3"] {
        app.configure(
            component,
            wasm_junction::WasiSettings::new()
                .env("GREETING", "hello")
                .arg("alpha"),
        )
        .unwrap();
    }
    block_on(app.load(Component::from_bytes(P2_COMPONENT).unwrap().named("p2"))).unwrap();
    block_on(app.load(Component::from_bytes(COMPONENT).unwrap().named("p3"))).unwrap();

    let environment =
        block_on(app.call("p2", P2_EXPORT, "read", vec![Val::from("GREETING")])).unwrap();
    let [Val::Option(Some(environment))] = environment.as_slice() else {
        panic!("Preview 2 environment returned the wrong shape")
    };
    let Val::String(environment) = environment.as_ref() else {
        panic!("Preview 2 environment value was not a string")
    };
    let arguments = block_on(app.call("p2", P2_EXPORT, "arguments", Vec::new())).unwrap();
    let [Val::List(arguments)] = arguments.as_slice() else {
        panic!("Preview 2 arguments returned the wrong shape")
    };
    let arguments: Vec<_> = arguments
        .iter()
        .map(|argument| match argument {
            Val::String(argument) => argument.as_str(),
            _ => panic!("Preview 2 argument was not a string"),
        })
        .collect();

    let result = block_on(app.call("p3", EXPORT, "cli", Vec::new())).unwrap();
    let [Val::String(result)] = result.as_slice() else {
        panic!("Preview 3 CLI probe returned the wrong shape")
    };
    let expected = format!("[(\"GREETING\", \"{environment}\")]|{arguments:?}|");
    assert!(result.starts_with(&expected), "{result}");
}

#[cfg(feature = "wasi-p3")]
type ChannelEvents = Arc<Mutex<Vec<(bool, wasm_junction::InvocationId, u64, ChannelDirection)>>>;
#[cfg(feature = "wasi-p3")]
type CapturedBytes = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

#[cfg(feature = "wasi-p3")]
fn record_channel_event(events: &ChannelEvents, event: &Event) {
    let observation = match event {
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
    if let Some(observation) = observation {
        events.lock().unwrap().push(observation);
    }
}

#[cfg(feature = "wasi-p3")]
fn assert_channel_closed(
    events: &ChannelEvents,
    invocation: wasm_junction::InvocationId,
    direction: ChannelDirection,
) {
    let events: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.1 == invocation && event.3 == direction)
        .copied()
        .collect();
    assert_eq!(events.len(), 2);
    assert!(events[0].0);
    assert_eq!(events[1], (false, invocation, events[0].2, direction));
}

#[cfg(feature = "wasi-p3")]
struct StdioPolicy {
    refuse_stdout: bool,
    bytes: CapturedBytes,
    calls: Arc<Mutex<Vec<wasm_junction::InvocationId>>>,
    events: ChannelEvents,
}

#[cfg(feature = "wasi-p3")]
impl Middleware for StdioPolicy {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.function.as_ref() != "write-via-stream" {
            return next.run(call).await;
        }
        self.calls.lock().unwrap().push(call.invocation_id());
        let Val::Stream(stream) = call.args.remove(0) else {
            return Err(CallError::trap("stdio did not carry a byte stream"));
        };
        let bytes = InputStream::try_from(stream)
            .map_err(|error| CallError::trap(error.to_string()))?
            .read_all()
            .await
            .map_err(|error| CallError::trap(error.to_string()))?;
        self.bytes
            .lock()
            .unwrap()
            .push((call.interface.to_string(), bytes.clone()));
        if self.refuse_stdout && call.interface.as_ref() == "wasi:cli/stdout@0.3.0" {
            return Err(CallError::refused("stdout denied"));
        }
        call.args
            .push(Val::Stream(StreamHandle::from(OutputStream::from_bytes(
                bytes,
            ))));
        next.run(call).await
    }

    fn event(&self, event: &Event) {
        record_channel_event(&self.events, event);
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn output_bytes_and_completion_cross_middleware() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let app = p3_app(StdioPolicy {
        refuse_stdout: false,
        bytes: bytes.clone(),
        calls: calls.clone(),
        events: events.clone(),
    });
    let result = block_on(app.call("p3", EXPORT, "cli", Vec::new())).unwrap();
    let [Val::String(result)] = result.as_slice() else {
        panic!("CLI probe returned the wrong shape")
    };
    assert!(result.contains("|0|ok|true|true|"));
    assert_eq!(
        *bytes.lock().unwrap(),
        [
            ("wasi:cli/stdout@0.3.0".to_owned(), b"stdout".to_vec()),
            ("wasi:cli/stderr@0.3.0".to_owned(), b"stderr".to_vec()),
        ]
    );

    let calls = calls.lock().unwrap();
    let events = events.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(events.len(), 6);
    for (open, invocation, stream, direction) in events.iter().filter(|event| event.0) {
        assert!(*open);
        assert!(calls.contains(invocation));
        assert!(events.contains(&(false, *invocation, *stream, *direction)));
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn output_refusal_resolves_to_the_cli_error() {
    let app = p3_app(StdioPolicy {
        refuse_stdout: true,
        bytes: Arc::new(Mutex::new(Vec::new())),
        calls: Arc::new(Mutex::new(Vec::new())),
        events: Arc::new(Mutex::new(Vec::new())),
    });
    let result = block_on(app.call("p3", EXPORT, "cli", Vec::new())).unwrap();
    let [Val::String(result)] = result.as_slice() else {
        panic!("CLI probe returned the wrong shape")
    };
    assert!(result.contains("|0|ok|false|true|"));
}

#[cfg(feature = "wasi-p3")]
struct RefuseStdin {
    invocation: Arc<Mutex<Option<wasm_junction::InvocationId>>>,
    events: ChannelEvents,
}

#[cfg(feature = "wasi-p3")]
impl Middleware for RefuseStdin {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == "wasi:cli/stdin@0.3.0" {
            *self.invocation.lock().unwrap() = Some(call.invocation_id());
            Err(CallError::refused("stdin denied"))
        } else {
            next.run(call).await
        }
    }

    fn event(&self, event: &Event) {
        record_channel_event(&self.events, event);
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn stdin_refusal_returns_a_closed_stream_and_io_completion() {
    let invocation = Arc::new(Mutex::new(None));
    let events = Arc::new(Mutex::new(Vec::new()));
    let app = p3_app(RefuseStdin {
        invocation: invocation.clone(),
        events: events.clone(),
    });
    let result = block_on(app.call("p3", EXPORT, "cli", Vec::new())).unwrap();
    let [Val::String(result)] = result.as_slice() else {
        panic!("CLI probe returned the wrong shape")
    };
    assert!(result.contains("|0|io|true|true|"));
    assert_channel_closed(
        &events,
        invocation.lock().unwrap().unwrap(),
        ChannelDirection::HostToGuest,
    );
}

#[cfg(feature = "wasi-p3")]
struct BlockOutputNext {
    claimed: AtomicBool,
    opened: AtomicBool,
    hold_guest: AtomicBool,
    state: Arc<OutputDropState>,
}

#[cfg(feature = "wasi-p3")]
#[derive(Default)]
struct OutputDropState {
    entered: Notify,
    progressed: Notify,
    waiting: Notify,
    writer: Mutex<Option<OutputStreamWriter>>,
    invocation: Mutex<Option<wasm_junction::InvocationId>>,
    events: ChannelEvents,
}

#[cfg(feature = "wasi-p3")]
impl Middleware for BlockOutputNext {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        let progressed = self.opened.load(Ordering::Relaxed)
            && call.interface.as_ref() == "wasi:cli/environment@0.3.0"
            && call.function.as_ref() == "get-arguments";
        if progressed {
            self.state.progressed.notify_one();
        }
        let hold = self.opened.load(Ordering::Relaxed)
            && call.interface.as_ref() == "wasi:clocks/monotonic-clock@0.3.0"
            && call.function.as_ref() == "wait-for";
        if hold && self.hold_guest.swap(false, Ordering::Relaxed) {
            self.state.waiting.notify_one();
            return std::future::pending().await;
        }
        let block = call.interface.as_ref() == "wasi:cli/stdout@0.3.0"
            && !self.claimed.swap(true, Ordering::Relaxed);
        if !block {
            return next.run(call).await;
        }
        *self.state.invocation.lock().unwrap() = Some(call.invocation_id());
        let (writer, stream) = OutputStream::channel();
        *self.state.writer.lock().unwrap() = Some(writer);
        call.args = vec![Val::Stream(StreamHandle::from(stream))];
        self.state.entered.notify_one();
        next.run(call).await
    }

    fn event(&self, event: &Event) {
        if matches!(
            event,
            Event::ChannelOpen {
                direction: ChannelDirection::GuestToHost,
                ..
            }
        ) {
            self.opened.store(true, Ordering::Relaxed);
        }
        record_channel_event(&self.state.events, event);
    }
}

#[cfg(feature = "wasi-p3")]
async fn assert_store_usable(app: &App) {
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        app.call("p3", EXPORT, "cli", Vec::new()),
    )
    .await
    .unwrap()
    .unwrap();
}

#[test]
#[cfg(feature = "wasi-p3")]
fn dropping_output_completion_finishes_cleanly() {
    let state = Arc::new(OutputDropState::default());
    let app = p3_app(BlockOutputNext {
        claimed: AtomicBool::new(false),
        opened: AtomicBool::new(false),
        hold_guest: AtomicBool::new(false),
        state: state.clone(),
    });
    block_on(async {
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut call = Box::pin(app.call("p3", EXPORT, "drop-output-completion", Vec::new()));
            let ready = async {
                tokio::join!(state.entered.notified(), state.progressed.notified());
            };
            tokio::pin!(ready);
            tokio::select! {
                biased;
                () = &mut ready => {}
                result = &mut call => panic!("output call resolved before guest progress: {result:?}"),
            }
            state.writer.lock().unwrap().take();
            call.await
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result, [Val::List(vec![Val::from("alpha")])]);
        let invocation = state.invocation.lock().unwrap().unwrap();
        assert_channel_closed(&state.events, invocation, ChannelDirection::GuestToHost);
        assert_store_usable(&app).await;
    });
}

#[test]
#[cfg(feature = "wasi-p3")]
fn canceling_invocation_inside_output_next_closes_the_channel() {
    let state = Arc::new(OutputDropState::default());
    let app = p3_app(BlockOutputNext {
        claimed: AtomicBool::new(false),
        opened: AtomicBool::new(false),
        hold_guest: AtomicBool::new(true),
        state: state.clone(),
    });
    block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut call = Box::pin(app.call("p3", EXPORT, "drop-output-completion", Vec::new()));
            let ready = async {
                tokio::join!(state.entered.notified(), state.waiting.notified());
            };
            tokio::pin!(ready);
            tokio::select! {
                biased;
                () = &mut ready => {}
                result = &mut call => panic!("blocked output call returned: {result:?}"),
            }
            drop(call);
            state.writer.lock().unwrap().take();
            let invocation = state.invocation.lock().unwrap().unwrap();
            loop {
                let closed = state.events.lock().unwrap().iter().any(|event| {
                    !event.0 && event.1 == invocation && event.3 == ChannelDirection::GuestToHost
                });
                if closed {
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert_channel_closed(&state.events, invocation, ChannelDirection::GuestToHost);
        })
        .await
        .unwrap();
        assert_store_usable(&app).await;
    });
}

#[cfg(feature = "wasi-p3")]
struct RefuseExit(Arc<Mutex<Option<(String, Vals)>>>);

#[cfg(feature = "wasi-p3")]
impl Middleware for RefuseExit {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == "wasi:cli/exit@0.3.0" {
            *self.0.lock().unwrap() = Some((call.function.to_string(), call.args.clone()));
            Err(CallError::refused("Preview 3 exit denied"))
        } else {
            next.run(call).await
        }
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn refused_exit_is_observed_and_ends_the_invocation() {
    let seen = Arc::new(Mutex::new(None));
    let app = p3_app(RefuseExit(seen.clone()));
    let error = block_on(app.call("p3", EXPORT, "exit-code", Vec::new())).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "Preview 3 exit denied");
    assert_eq!(
        *seen.lock().unwrap(),
        Some(("exit-with-code".to_owned(), vec![Val::U8(7)]))
    );
}

#[cfg(feature = "wasi-p3")]
enum RewriteTerminalInput {
    Foreign(Mutex<Option<wasm_junction::Resource>>),
    Mistyped,
    Unscoped,
}

#[cfg(feature = "wasi-p3")]
impl Middleware for RewriteTerminalInput {
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == "wasi:cli/terminal-input@0.3.0"
            && call.function.as_ref() == "[drop]terminal-input"
        {
            let Val::Resource(current) = &call.args[0] else {
                panic!("terminal drop did not receive a resource");
            };
            let replacement = match self {
                Self::Foreign(saved) => {
                    let mut saved = saved.lock().unwrap();
                    if let Some(foreign) = saved.as_ref() {
                        foreign.clone()
                    } else {
                        *saved = Some(current.clone());
                        return Ok(Vec::new());
                    }
                }
                Self::Mistyped => wasm_junction::Resource::__owned_for_invocation(
                    "wasi:cli/terminal-output@0.3.0",
                    "terminal-output",
                    current.id(),
                    call.invocation_id(),
                ),
                Self::Unscoped => wasm_junction::Resource::owned(
                    current.interface(),
                    current.name(),
                    current.id(),
                ),
            };
            call.args[0] = Val::Resource(replacement);
        }
        let invocation = call.invocation_id();
        let terminal_stdin = call.interface.as_ref() == "wasi:cli/terminal-stdin@0.3.0"
            && call.function.as_ref() == "get-terminal-stdin";
        let mut values = next.run(call).await?;
        if terminal_stdin {
            values = vec![Val::Option(Some(Box::new(Val::Resource(
                wasm_junction::Resource::__owned_for_invocation(
                    "wasi:cli/terminal-input@0.3.0",
                    "terminal-input",
                    23,
                    invocation,
                ),
            ))))];
        }
        Ok(values)
    }
}

#[cfg(feature = "wasi-p3")]
fn assert_invalid_terminal_handle(middleware: RewriteTerminalInput, expected: &str) {
    let app = p3_app(middleware);
    let error = block_on(app.call("p3", EXPORT, "cli", Vec::new())).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(error.to_string().contains(expected));
}

#[test]
#[cfg(feature = "wasi-p3")]
fn foreign_terminal_handle_is_refused() {
    let app = p3_app(RewriteTerminalInput::Foreign(Mutex::new(None)));
    block_on(app.call("p3", EXPORT, "cli", Vec::new())).unwrap();
    let error = block_on(app.call("p3", EXPORT, "cli", Vec::new())).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(
        error
            .to_string()
            .contains("does not belong to this invocation")
    );
}

#[test]
#[cfg(feature = "wasi-p3")]
fn mistyped_terminal_handle_is_refused() {
    assert_invalid_terminal_handle(
        RewriteTerminalInput::Mistyped,
        "does not match the resource type",
    );
}

#[test]
#[cfg(feature = "wasi-p3")]
fn unscoped_terminal_handle_is_refused() {
    assert_invalid_terminal_handle(
        RewriteTerminalInput::Unscoped,
        "does not belong to this invocation",
    );
}

#[cfg(feature = "wasi-p3")]
struct RecordTerminalDrops(Arc<Mutex<Vec<String>>>);

#[cfg(feature = "wasi-p3")]
impl Middleware for RecordTerminalDrops {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if matches!(
            call.function.as_ref(),
            "[drop]terminal-input" | "[drop]terminal-output"
        ) {
            self.0.lock().unwrap().push(call.function.to_string());
            return Ok(Vec::new());
        }
        let invocation = call.invocation_id();
        let terminal = match call.interface.as_ref() {
            "wasi:cli/terminal-stdin@0.3.0" => {
                Some(("wasi:cli/terminal-input@0.3.0", "terminal-input", 31))
            }
            "wasi:cli/terminal-stdout@0.3.0" | "wasi:cli/terminal-stderr@0.3.0" => {
                Some(("wasi:cli/terminal-output@0.3.0", "terminal-output", 32))
            }
            _ => None,
        };
        let mut values = next.run(call).await?;
        if let Some((interface, name, id)) = terminal {
            values = vec![Val::Option(Some(Box::new(Val::Resource(
                wasm_junction::Resource::__owned_for_invocation(interface, name, id, invocation),
            ))))];
        }
        Ok(values)
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn every_terminal_resource_drop_crosses_middleware() {
    let drops = Arc::new(Mutex::new(Vec::new()));
    let app = p3_app(RecordTerminalDrops(drops.clone()));
    let result = block_on(app.call("p3", EXPORT, "cli", Vec::new())).unwrap();
    let [Val::String(result)] = result.as_slice() else {
        panic!("CLI probe returned the wrong shape")
    };
    assert!(result.ends_with("(true, true, true)"));
    let mut drops = drops.lock().unwrap().clone();
    drops.sort();
    assert_eq!(
        drops,
        [
            "[drop]terminal-input",
            "[drop]terminal-output",
            "[drop]terminal-output",
        ]
    );
}

#[test]
#[cfg(feature = "wasi-p3")]
fn p3_clock_waits_can_await_middleware_and_be_rewritten() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = p3_app(WaitBehavior {
        refuse: false,
        calls: calls.clone(),
    });
    let values = block_on(app.call("p3", EXPORT, "coverage", Vec::new())).unwrap();
    assert_eq!(values, [Val::from("true|true|4|5|true|true")]);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
#[cfg(feature = "wasi-p3")]
fn refusal_of_a_p3_function_without_an_error_result_traps() {
    let app = p3_app(WaitBehavior {
        refuse: true,
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let error = block_on(app.call("p3", EXPORT, "coverage", Vec::new())).unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert_eq!(error.to_string(), "p3 wait denied");
}

#[cfg(feature = "wasi-p3")]
struct RecordGates(Arc<Mutex<BTreeSet<(String, String)>>>);

#[cfg(feature = "wasi-p3")]
impl Middleware for RecordGates {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.starts_with("wasi:") {
            self.0
                .lock()
                .unwrap()
                .insert((call.interface.to_string(), call.function.to_string()));
        }
        next.run(call).await
    }
}

#[test]
#[cfg(feature = "wasi-p3")]
fn every_function_in_each_gated_p3_interface_has_a_gate() {
    let seen = Arc::new(Mutex::new(BTreeSet::new()));
    let app = p3_app(RecordGates(seen.clone()));
    block_on(app.call("p3", EXPORT, "coverage", Vec::new())).unwrap();
    block_on(app.call("p3", EXPORT, "exit-success", Vec::new())).unwrap_err();
    block_on(app.call("p3", EXPORT, "exit-code", Vec::new())).unwrap_err();
    let directory = TestDirectory::new("coverage");
    let filesystem = filesystem_app(RecordGates(seen.clone()), &directory);
    block_on(filesystem.call("p3", EXPORT, "filesystem-coverage", Vec::new())).unwrap();
    let sockets = sockets_app(RecordGates(seen.clone()));
    block_on(sockets.call("p3", EXPORT, "sockets-coverage", Vec::new())).unwrap();

    let mut expected = p3_wit_functions();
    expected.insert((
        "wasi:filesystem/types@0.3.0".to_owned(),
        "[drop]descriptor".to_owned(),
    ));
    expected.insert((
        "wasi:sockets/types@0.3.0".to_owned(),
        "[drop]tcp-socket".to_owned(),
    ));
    expected.insert((
        "wasi:sockets/types@0.3.0".to_owned(),
        "[drop]udp-socket".to_owned(),
    ));
    assert_eq!(*seen.lock().unwrap(), expected);
}

#[cfg(feature = "wasi-p3")]
fn p3_wit_functions() -> BTreeSet<(String, String)> {
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
        .filter(|package| package["name"] == "wasmtime-wasi")
        .map(|package| package["manifest_path"].as_str().unwrap())
        .collect();
    assert_eq!(manifests.len(), 1, "expected one resolved wasmtime-wasi");
    let wit = Path::new(manifests[0]).parent().unwrap().join("src/p3/wit");
    let mut resolve = wit_parser::Resolve::default();
    resolve.push_dir(wit).unwrap();

    resolve
        .packages
        .iter()
        .flat_map(|(_, package)| {
            package.interfaces.iter().filter_map(|(name, interface)| {
                let name = package.name.interface_id(name);
                WASI_INTERFACES
                    .contains(&name.as_str())
                    .then_some((name, &resolve.interfaces[*interface]))
            })
        })
        .flat_map(|(interface, definition)| {
            definition
                .functions
                .keys()
                .map(move |function| (interface.clone(), function.clone()))
        })
        .collect()
}

#[test]
#[cfg(feature = "wasi-p3")]
fn ungated_p3_interfaces_remain_missing() {
    let app = App::builder()
        .engine(WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .build()
        .unwrap();

    let name = "wasi:http/types@0.3.0";
    let component = Component::from_bytes(component_importing("http", "types"))
        .unwrap()
        .named("http");
    let LoadError::MissingImports(missing) = block_on(app.load(component)).unwrap_err() else {
        panic!("expected {name} to remain missing");
    };
    assert_eq!(missing.interfaces(), [name]);
}

#[cfg(feature = "wasi-p3")]
fn component_importing(package: &str, interface: &str) -> Vec<u8> {
    let wit = format!(
        "package wasi:{package}@0.3.0; interface {interface} {{ probe: func(); }} world fixture {{ import {interface}; }}"
    );
    let mut resolve = Resolve::default();
    let package = resolve.push_str("fixture.wit", &wit).unwrap();
    let world = resolve.select_world(&[package], Some("fixture")).unwrap();
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}
