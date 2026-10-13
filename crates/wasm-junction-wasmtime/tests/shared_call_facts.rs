//! Request-scoped facts across routed component and WASI calls.

#![cfg(feature = "wasi-http")]
#![forbid(unsafe_code)]

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use support::HttpServer;
use wasm_junction::{Access, App, Call, CallError, Component, Middleware, Next, Val, Vals};

wasm_junction::bindgen!({
    path: "test-fixtures/call-facts/wit",
    interfaces: ["test:call-facts/agent@0.1.0"],
});

const AGENT_COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/call-facts-test.wasm"));
const NOTES_TOOLS_COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wasi-test.wasm"));
const NOTES_TOOLS_INTERFACE: &str = "test:wasi/environment@0.1.0";
const FILESYSTEM: &str = "wasi:filesystem/types@0.2.12";
const HTTP: &str = "wasi:http/outgoing-handler@0.2.12";
static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Default)]
struct Facts(Arc<Mutex<FactState>>);

#[derive(Default)]
struct FactState {
    calls: Vec<CallFact>,
    refusal: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct CallFact {
    caller: String,
    callee: String,
    interface: String,
    function: String,
    context: Option<String>,
}

#[derive(Default)]
struct Policy {
    rendezvous: Option<Arc<Barrier>>,
}

impl Middleware for Policy {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let facts = call.extensions().get::<Facts>().cloned();
        if let Some(facts) = &facts {
            facts.0.lock().unwrap().calls.push(CallFact {
                caller: call.caller.to_string(),
                callee: call.callee.to_string(),
                interface: call.interface.to_string(),
                function: call.function.to_string(),
                context: policy_context(&call),
            });
        }
        let outgoing = call.interface.as_ref() == HTTP && call.function.as_ref() == "handle";
        if outgoing && let Some(rendezvous) = &self.rendezvous {
            rendezvous.wait();
        }
        if outgoing && facts.as_ref().is_some_and(private_note_was_read) {
            let reason = "tainted by `/notes/private/diary.md`".to_owned();
            facts.unwrap().0.lock().unwrap().refusal = Some(reason.clone());
            return Err(CallError::refused(reason));
        }
        next.run(call).await
    }
}

fn private_note_was_read(facts: &Facts) -> bool {
    facts.0.lock().unwrap().calls.iter().any(|fact| {
        fact.interface == FILESYSTEM
            && fact.function == "[method]descriptor.open-at"
            && fact.context.as_deref() == Some("/notes/private/diary.md")
    })
}

fn policy_context(call: &Call) -> Option<String> {
    if call.interface.as_ref() == FILESYSTEM
        && call.function.as_ref() == "[method]descriptor.open-at"
    {
        let (Some(Val::String(path)), Some(Val::String(root))) =
            (call.args.get(2), call.args.last())
        else {
            return None;
        };
        return Some(format!("{}/{path}", root.trim_end_matches('/')));
    }
    (call.interface.as_ref() == HTTP && call.function.as_ref() == "handle")
        .then(|| optional_string(call.args.get(3)))
        .flatten()
}

fn optional_string(value: Option<&Val>) -> Option<String> {
    let Some(Val::Option(Some(value))) = value else {
        return None;
    };
    let Val::String(value) = value.as_ref() else {
        return None;
    };
    Some(value.clone())
}

struct NotesDirectory(PathBuf);

impl NotesDirectory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("wasm-junction-facts-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("private")).unwrap();
        std::fs::create_dir_all(path.join("public")).unwrap();
        std::fs::write(path.join("private/diary.md"), "private note").unwrap();
        std::fs::write(path.join("public/brief.md"), "public note").unwrap();
        Self(path)
    }
}

impl Drop for NotesDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn loaded(notes: &NotesDirectory, policy: impl Middleware + 'static) -> (App, agent::Agent) {
    let app = App::builder()
        .engine(wasm_junction_wasmtime::WasmtimeEngine::new().unwrap())
        .provide(wasm_junction::wasi::provider())
        .provide(wasm_junction::wasi::http::provider())
        .middleware(policy)
        .build()
        .unwrap();
    app.configure(
        "notes-tools",
        wasm_junction::WasiSettings::new().preopen(&notes.0, "/notes", Access::ReadOnly),
    )
    .unwrap();
    app.configure("agent", wasm_junction::WasiSettings::new().network(true))
        .unwrap();
    block_on(
        app.load_all([
            Component::from_bytes(NOTES_TOOLS_COMPONENT)
                .unwrap()
                .named("notes-tools"),
            Component::from_bytes(AGENT_COMPONENT)
                .unwrap()
                .named("agent"),
        ]),
    )
    .unwrap();
    let agent = app.get::<agent::Agent>("agent").unwrap();
    (app, agent)
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn facts_span_routed_component_and_wasi_calls() {
    let notes = NotesDirectory::new();
    let (_app, agent) = loaded(&notes, Policy::default());
    let facts = Facts::default();
    let server = HttpServer::start(b"ignored");
    let authority = server.authority();

    let result = block_on(
        agent
            .with(facts.clone())
            .run("/notes/public/brief.md", &authority),
    )
    .unwrap();
    assert_eq!(result, "public note|sent");
    assert!(server.finish().starts_with("GET /research HTTP/1.1\r\n"));

    let state = facts.0.lock().unwrap();
    assert!(state.calls.iter().any(|fact| fact
        == &CallFact {
            caller: "agent".into(),
            callee: "notes-tools".into(),
            interface: NOTES_TOOLS_INTERFACE.into(),
            function: "read-file".into(),
            context: None,
        }));
    let file = state
        .calls
        .iter()
        .position(|fact| {
            fact.interface == FILESYSTEM
                && fact.context.as_deref() == Some("/notes/public/brief.md")
        })
        .unwrap();
    let http = state
        .calls
        .iter()
        .position(|fact| fact.interface == HTTP && fact.context.as_deref() == Some(&authority))
        .unwrap();
    assert!(file < http);
}

#[test]
fn facts_from_a_routed_read_refuse_a_later_http_call() {
    let notes = NotesDirectory::new();
    let (_app, agent) = loaded(&notes, Policy::default());
    let facts = Facts::default();

    let result = block_on(
        agent
            .with(facts.clone())
            .run("/notes/private/diary.md", "private.example"),
    )
    .unwrap();
    assert_eq!(result, "private note|denied");
    assert_eq!(
        facts.0.lock().unwrap().refusal.as_deref(),
        Some("tainted by `/notes/private/diary.md`")
    );

    let server = HttpServer::start(b"ignored");
    let result = block_on(agent.run("/notes/private/diary.md", &server.authority())).unwrap();
    assert_eq!(result, "private note|sent");
    assert!(server.finish().starts_with("GET /research HTTP/1.1\r\n"));
}

#[test]
fn concurrent_requests_keep_fact_stores_isolated() {
    let notes = NotesDirectory::new();
    let policy = Policy {
        rendezvous: Some(Arc::new(Barrier::new(2))),
    };
    let (_app, agent) = loaded(&notes, policy);
    let private_facts = Facts::default();
    let public_facts = Facts::default();
    let private = agent.with(private_facts.clone());
    let public = agent.with(public_facts.clone());
    let server = HttpServer::start(b"ignored");
    let public_authority = server.authority();

    let (private_result, public_result) = std::thread::scope(|scope| {
        let private = scope
            .spawn(|| block_on(private.run("/notes/private/diary.md", "private.example")).unwrap());
        let public = scope
            .spawn(|| block_on(public.run("/notes/public/brief.md", &public_authority)).unwrap());
        (private.join().unwrap(), public.join().unwrap())
    });

    assert_eq!(private_result, "private note|denied");
    assert_eq!(public_result, "public note|sent");
    assert!(server.finish().starts_with("GET /research HTTP/1.1\r\n"));
    assert!(private_note_was_read(&private_facts));
    assert!(!private_note_was_read(&public_facts));
    let private = private_facts.0.lock().unwrap();
    let public = public_facts.0.lock().unwrap();
    assert_eq!(
        private.refusal.as_deref(),
        Some("tainted by `/notes/private/diary.md`")
    );
    assert_eq!(public.refusal, None);
    assert!(private.calls.iter().any(|fact| {
        fact.interface == HTTP && fact.context.as_deref() == Some("private.example")
    }));
    assert!(public.calls.iter().any(|fact| {
        fact.interface == FILESYSTEM && fact.context.as_deref() == Some("/notes/public/brief.md")
    }));
    assert!(public.calls.iter().any(|fact| {
        fact.interface == HTTP && fact.context.as_deref() == Some(&public_authority)
    }));
    assert!(!public.calls.iter().any(|fact| {
        fact.context.as_deref() == Some("/notes/private/diary.md")
            || fact.context.as_deref() == Some("private.example")
    }));
}
