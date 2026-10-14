use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wasm_junction::{Access, Call, CallError, Middleware, Next, Vals};

use super::{Console, Settings, repl, weather_wasi};
use crate::test_support::SharedOutput;

#[derive(Clone, Default)]
struct Reached {
    notes_tools: Arc<AtomicBool>,
}

impl Middleware for Reached {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == "example:assistant/notes-tools@0.1.0" {
            self.notes_tools.store(true, Ordering::Relaxed);
        }
        next.run(call).await
    }
}

#[test]
fn weather_can_attempt_reads_but_cannot_write_notes() {
    let settings = weather_wasi();
    let (_, guest, access) = settings.preopens().next().unwrap();
    assert_eq!((guest, access), ("/notes", Access::ReadOnly));
}

#[tokio::test(flavor = "current_thread")]
async fn refuses_consent_then_continues_chatting() {
    if std::env::var_os("OPENAI_BASE_URL").is_none() {
        eprintln!("skipped: set OPENAI_BASE_URL to run the assistant end-to-end test");
        return;
    }
    let mut settings = Settings::read().unwrap().unwrap();
    settings.private_data_hosts.clear();
    let input = "Use read_note to read private/diary.md, then summarize it.\nn\nReply with one short greeting without tools.\n\n";
    let reached = Reached::default();
    let observer = reached.clone();
    let output = SharedOutput::default();
    repl(
        &settings,
        Console::new(Cursor::new(input), output.clone()),
        false,
        |builder| builder.middleware(observer),
    )
    .await
    .unwrap();
    let output = output.text();
    assert!(output.contains("[y/N]"));
    assert!(output.contains("assistant error>"));
    assert!(output.contains("private/diary.md"));
    assert!(
        output
            .contains("  ? This conversation includes private/diary.md. Let it make a request to ")
    );
    assert_eq!(output.matches("assistant> ").count(), 1);
    assert!(reached.notes_tools.load(Ordering::Relaxed));
}
