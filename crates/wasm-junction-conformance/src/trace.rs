use std::sync::{Arc, Mutex, MutexGuard};

use wasm_junction::{Call, Event, Middleware, Next, Trap, Val, Vals};

/// Middleware that records calls, returns, traps, and invocation boundaries.
#[derive(Clone, Default)]
pub struct Trace(Arc<Mutex<Vec<String>>>);

impl Trace {
    /// Returns a snapshot of the recorded entries.
    #[must_use]
    pub fn entries(&self) -> Vec<String> {
        self.lock().clone()
    }

    /// Removes all recorded entries.
    pub fn clear(&self) {
        self.lock().clear();
    }

    fn record(&self, entry: String) {
        self.lock().push(entry);
    }

    fn lock(&self) -> MutexGuard<'_, Vec<String>> {
        match self.0.lock() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Middleware for Trace {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, Trap> {
        let label = call.to_string();
        self.record(format!("call {label}({})", vals(&call.args)));
        let result = next.run(call).await;
        match &result {
            Ok(values) => self.record(format!("return {label}({})", vals(values))),
            Err(error) => self.record(format!("trap {label}({error})")),
        }
        result
    }

    fn event(&self, event: &Event) {
        match event {
            Event::InvocationStart { component } => {
                self.record(format!("invocation start {component}"));
            }
            Event::InvocationEnd { component } => {
                self.record(format!("invocation end {component}"));
            }
            _ => {}
        }
    }
}

fn vals(values: &[Val]) -> String {
    values.iter().map(val).collect::<Vec<_>>().join(", ")
}

fn val(value: &Val) -> String {
    match value {
        Val::Bool(value) => value.to_string(),
        Val::S8(value) => value.to_string(),
        Val::U8(value) => value.to_string(),
        Val::S16(value) => value.to_string(),
        Val::U16(value) => value.to_string(),
        Val::S32(value) => value.to_string(),
        Val::U32(value) => value.to_string(),
        Val::S64(value) => value.to_string(),
        Val::U64(value) => value.to_string(),
        Val::F32(value) => value.to_string(),
        Val::F64(value) => value.to_string(),
        Val::Char(value) => format!("{value:?}"),
        Val::String(value) => format!("{value:?}"),
        Val::List(values) => format!("[{}]", vals(values)),
        Val::Tuple(values) => format!("({})", vals(values)),
        Val::Record(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(name, value)| format!("{name}: {}", val(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Val::Variant { case, value } => value.as_deref().map_or_else(
            || (*case).to_owned(),
            |value| format!("{case}({})", val(value)),
        ),
        Val::Enum(case) => (*case).to_owned(),
        Val::Flags(names) => format!("[{}]", names.join("|")),
        Val::Option(value) => value.as_deref().map_or_else(
            || "none".to_owned(),
            |value| format!("some({})", val(value)),
        ),
        Val::Result(result) => match result {
            Ok(value) => result_value("ok", value.as_deref()),
            Err(value) => result_value("err", value.as_deref()),
        },
        _ => format!("{value:?}"),
    }
}

fn result_value(case: &str, value: Option<&Val>) -> String {
    value.map_or_else(
        || case.to_owned(),
        |value| format!("{case}({})", val(value)),
    )
}

macro_rules! with_note {
    ($prefix:literal, $suffix:literal) => {
        concat!(
            $prefix,
            "{title: \"Daily\", published: true, signed-8: -8, unsigned-8: 8, ",
            "signed-16: -16, unsigned-16: 16, signed-32: -32, unsigned-32: 32, ",
            "signed-64: -64, unsigned-64: 64, score-32: 3.5, score-64: 7.25, ",
            "marker: '§', tags: [\"rust\", \"wasm\"], location: (-71, 42), ",
            "attachment: text(\"diagram\"), mood: upbeat, emphasis: [concise|detailed], ",
            "subtitle: some(\"Engine notes\"), revision: ok(7)}",
            $suffix
        )
    };
}

#[cfg(test)]
const NOTE: &str = with_note!("", "");

/// Exact trace produced by the fixture's successful summary scenario.
pub const EXPECTED_TRACE: &[&str] = &[
    "invocation start summarizer",
    "call host → summarizer example:notes/summarizer@0.1.0.summarize(\"daily\")",
    "invocation start host",
    "call summarizer → host example:notes/notes@0.1.0.read(\"daily\")",
    with_note!(
        "return summarizer → host example:notes/notes@0.1.0.read(ok(",
        "))"
    ),
    "invocation end host",
    "invocation start host",
    with_note!(
        "call summarizer → host example:notes/notes@0.1.0.normalize(",
        ")"
    ),
    with_note!(
        "return summarizer → host example:notes/notes@0.1.0.normalize(",
        ")"
    ),
    "invocation end host",
    with_note!(
        "return host → summarizer example:notes/summarizer@0.1.0.summarize(ok({text: \"Daily: 2 tags\", source: ",
        "}))"
    ),
    "invocation end summarizer",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sample_note, sample_summary};

    #[test]
    fn expected_trace_covers_calls_returns_and_boundaries() {
        assert_eq!(val(&sample_note()), NOTE);
        assert!(val(&sample_summary()).starts_with("ok({text:"));
        assert_eq!(EXPECTED_TRACE.len(), 12);
    }
}
