use std::io::{Cursor, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use wasm_junction::{Call, CallError, Middleware, Next, Vals};

use super::{Console, Server, Settings, repl};

#[derive(Clone, Default)]
struct Reached {
    notes_tools: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
struct SharedOutput(Arc<Mutex<Vec<u8>>>);

impl SharedOutput {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for SharedOutput {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Middleware for Reached {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        if call.interface.as_ref() == "example:assistant/notes-tools@0.1.0" {
            self.notes_tools.store(true, Ordering::Relaxed);
        }
        next.run(call).await
    }
}

#[tokio::test(flavor = "current_thread")]
async fn refuses_consent_then_continues_chatting() {
    if std::env::var_os("OPENAI_BASE_URL").is_none() {
        eprintln!("skipped: set OPENAI_BASE_URL to run the assistant end-to-end test");
        return;
    }
    let mut settings = Settings::read().unwrap().unwrap();
    settings.private_data_hosts.clear();
    let server = Server::start().unwrap();
    let input = "Use read_note to read private/diary.md, then summarize it.\nn\nReply with one short greeting without tools.\ny\n\n";
    let reached = Reached::default();
    let observer = reached.clone();
    let output = SharedOutput::default();
    repl(
        &settings,
        &server,
        Console::new(Cursor::new(input), output.clone()),
        |builder| builder.middleware(observer),
    )
    .await
    .unwrap();
    let output = output.text();
    assert!(output.contains("[y/N]"));
    assert!(output.contains("assistant error>"));
    assert!(output.contains("private/diary.md"));
    assert!(output.contains("asked → no"));
    assert_eq!(output.matches("assistant> ").count(), 1);
    assert!(reached.notes_tools.load(Ordering::Relaxed));
}
