//! Transforms byte and typed streams as they cross middleware.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bindings;
mod protection;

use std::error::Error;
use std::sync::{Arc, Mutex, MutexGuard};

use bindings::{runner, support};
use protection::ProtectTickets;
use wasm_junction::{App, CallContext, CallError, Component, InputStream, OutputStream};

const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/streams.wasm"));

#[derive(Default)]
struct Support {
    transcript: Mutex<String>,
}

impl Support {
    fn transcript(&self) -> MutexGuard<'_, String> {
        self.transcript
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl support::Host for Support {
    async fn store_transcript(
        &self,
        _cx: &CallContext,
        transcript: InputStream,
    ) -> Result<(), CallError> {
        *self.transcript() = String::from_utf8(transcript.read_all().await?)
            .map_err(|error| CallError::trap(error.to_string()))?;
        Ok(())
    }

    fn tickets(&self, _cx: &CallContext) -> Result<OutputStream<String>, CallError> {
        Ok(OutputStream::from_items([
            "public: Password reset".to_owned(),
            "private: Executive account incident".to_owned(),
            "public: Billing question".to_owned(),
        ]))
    }
}

/// Runs the support-ticket workflow and writes its complete output.
///
/// # Errors
///
/// Returns an error when the application cannot be built, loaded, or called.
pub async fn run(write: impl Fn(String) + Send + Sync + 'static) -> Result<(), Box<dyn Error>> {
    let support = Arc::new(Support::default());
    let app = App::builder()
        .provide(support::provider(support.clone()))
        .middleware(ProtectTickets)
        .build()?;
    app.load(Component::from_bytes(COMPONENT)?.named("support-plugin"))
        .await?;

    let summary = app.get::<runner::Runner>("support-plugin")?.run().await?;
    for line in support.transcript().lines() {
        write(format!("host stored: {line}"));
    }
    write(format!("guest summarised: {summary}"));
    Ok(())
}

#[cfg(all(test, target_family = "wasm"))]
mod browser_tests {
    use std::sync::{Arc, Mutex};

    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_dedicated_worker);

    #[wasm_bindgen_test]
    async fn browser_output_matches_native_output() {
        let actual = Arc::new(Mutex::new(String::new()));
        let captured = actual.clone();
        super::run(move |line| {
            let mut output = captured.lock().unwrap();
            output.push_str(&line);
            output.push('\n');
        })
        .await
        .unwrap();
        assert_eq!(
            *actual.lock().unwrap(),
            include_str!("../tests/expected-output.txt")
        );
    }
}
