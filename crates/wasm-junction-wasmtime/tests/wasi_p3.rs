//! WASI Preview 3 feature and gate behavior.

#![cfg(feature = "wasi")]

use std::future::Future;
#[cfg(feature = "wasi-p3")]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(feature = "wasi-p3")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "wasi-p3")]
use std::{collections::BTreeSet, path::Path, process::Command};

#[cfg(feature = "wasi-p3")]
use tokio::sync::Notify;
use wasm_junction::LoadError;
use wasm_junction::{App, Component};
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

    assert_eq!(*seen.lock().unwrap(), p3_wit_functions());
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

    for (package, interface) in [
        ("filesystem", "types"),
        ("sockets", "tcp"),
        ("http", "types"),
    ] {
        let name = format!("wasi:{package}/{interface}@0.3.0");
        let component = Component::from_bytes(component_importing(package, interface))
            .unwrap()
            .named(package);
        let LoadError::MissingImports(missing) = block_on(app.load(component)).unwrap_err() else {
            panic!("expected {name} to remain missing");
        };
        assert_eq!(missing.interfaces(), [name]);
    }
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
