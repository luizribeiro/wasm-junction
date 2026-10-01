use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use wasm_junction::{CallContext, Provided, Val};

use crate::{notes, types};

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
    ) -> impl std::future::Future<Output = Result<types::Note, String>> {
        std::future::ready(read(&name))
    }

    fn normalize(&self, _context: &CallContext, value: types::Note) -> types::Note {
        self.normalizations.fetch_add(1, Ordering::Relaxed);
        value
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
