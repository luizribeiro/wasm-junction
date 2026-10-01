use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, MutexGuard};

use wasm_junction::{CallContext, CallError, Caller, Provided, Val};

use crate::{SessionId, TranslatorHop, decoration, notes, types};

/// The host implementation used by the notes-summary fixture.
#[derive(Clone, Default)]
pub struct FixtureHost {
    normalizations: Arc<AtomicUsize>,
}

impl FixtureHost {
    /// Wraps this host as the fixture's notes provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        notes::provider(self)
    }

    /// Returns the number of notes normalized by this host.
    #[must_use]
    pub fn normalizations(&self) -> usize {
        self.normalizations.load(Ordering::Relaxed)
    }
}

impl notes::Host for FixtureHost {
    fn read(
        &self,
        _context: &CallContext,
        name: String,
    ) -> impl std::future::Future<Output = Result<Result<types::Note, String>, CallError>> {
        std::future::ready(if name == "provider-refusal" {
            Err(CallError::refused("notes provider refused the call"))
        } else {
            Ok(read(&name))
        })
    }

    fn normalize(
        &self,
        _context: &CallContext,
        value: types::Note,
    ) -> Result<types::Note, CallError> {
        self.normalizations.fetch_add(1, Ordering::Relaxed);
        Ok(value)
    }
}

/// The host provider called by the translator in the routed fixture.
#[derive(Clone, Default)]
pub struct RoutedHost(Arc<std::sync::Mutex<Vec<Caller>>>);

impl RoutedHost {
    /// Wraps this host as the fixture's decoration provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        decoration::provider(self)
    }

    /// Returns the callers observed by the provider.
    #[must_use]
    pub fn callers(&self) -> Vec<Caller> {
        self.lock().clone()
    }

    fn decorate(&self, context: &CallContext, text: &str) -> String {
        self.lock().push(context.caller().clone());
        let session = context.extensions().get::<SessionId>().map(|value| value.0);
        let hop = if context.extensions().get::<TranslatorHop>().is_some() {
            Some("writer-to-translator")
        } else {
            None
        };
        match (session, hop) {
            (None, None) => format!("host: {text}"),
            (session, hop) => format!(
                "host[session={}, hop={}]: {text}",
                session.map_or_else(|| "missing".to_owned(), |value| value.to_string()),
                hop.unwrap_or("missing")
            ),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Caller>> {
        match self.0.lock() {
            Ok(callers) => callers,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl decoration::Host for RoutedHost {
    fn decorate(&self, context: &CallContext, text: String) -> Result<String, CallError> {
        Ok(self.decorate(context, &text))
    }

    fn decorate_async(
        &self,
        context: &CallContext,
        text: String,
    ) -> impl std::future::Future<Output = Result<String, CallError>> {
        std::future::ready(Ok(self.decorate(context, &text)))
    }
}

fn read(name: &str) -> Result<types::Note, String> {
    if name == "private" {
        Err("permission denied".to_owned())
    } else {
        Ok(note())
    }
}

/// Returns the fixture note, containing every plain WIT value shape.
#[must_use]
pub fn sample_note() -> Val {
    note().into()
}

/// Returns the fixture's successful summary result.
#[must_use]
pub fn sample_summary() -> Val {
    Val::Result(Ok(Some(Box::new(summary().into()))))
}

pub(crate) fn note() -> types::Note {
    types::Note {
        title: "Daily".to_owned(),
        published: true,
        signed_8: -8,
        unsigned_8: 8,
        signed_16: -16,
        unsigned_16: 16,
        signed_32: -32,
        unsigned_32: 32,
        signed_64: -64,
        unsigned_64: 64,
        score_32: 3.5,
        score_64: 7.25,
        marker: '§',
        tags: vec!["rust".to_owned(), "wasm".to_owned()],
        location: (-71, 42),
        attachment: types::Attachment::Text("diagram".to_owned()),
        mood: types::Mood::Upbeat,
        emphasis: types::Emphasis {
            concise: true,
            detailed: true,
        },
        subtitle: Some("Engine notes".to_owned()),
        revision: Ok(7),
    }
}

pub(crate) fn summary() -> types::Summary {
    types::Summary {
        text: "Daily: 2 tags".to_owned(),
        source: note(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_is_a_typed_result() {
        assert_eq!(read("private"), Err("permission denied".to_owned()));
    }
}
