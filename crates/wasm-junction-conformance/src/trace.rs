use std::sync::{Arc, Mutex, MutexGuard};

use wasm_junction::{Call, CallError, Event, Middleware, Next, Val, Vals};

use crate::{SessionId, TranslatorHop};

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
    async fn call(&self, mut call: Call, next: Next) -> Result<Vals, CallError> {
        if matches!(&call.caller, wasm_junction::Caller::Component(name) if name.as_ref() == "writer")
            && call.callee.as_ref() == "translator"
        {
            call.extensions_mut().insert(TranslatorHop);
        }
        let label = format!("{call}{}", call_data(&call));
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
            Event::ResourceDrop {
                interface,
                resource,
                id,
            } => self.record(format!("resource drop {interface}/{resource}#{id}")),
            _ => {}
        }
    }
}

fn call_data(call: &Call) -> String {
    let mut values = Vec::new();
    if let Some(session) = call.extensions().get::<SessionId>() {
        values.push(format!("session={}", session.0));
    }
    if call.extensions().get::<TranslatorHop>().is_some() {
        values.push("hop=writer-to-translator".to_owned());
    }
    if values.is_empty() {
        String::new()
    } else {
        format!(" [{}]", values.join(", "))
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
        Val::Variant { case, value } => value
            .as_deref()
            .map_or_else(|| case.clone(), |value| format!("{case}({})", val(value))),
        Val::Enum(case) => case.clone(),
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

/// Exact trace produced by the successful routed-call scenario.
pub const EXPECTED_ROUTED_TRACE: &[&str] = &[
    "invocation start writer",
    "call host → writer example:notes/writer@0.1.0.write [session=42](\"hello\")",
    "invocation start translator",
    "call writer → translator example:notes/translator@0.1.0.translate [session=42, hop=writer-to-translator](\"hello\")",
    "invocation start host",
    "call translator → host example:notes/decoration@0.1.0.decorate [session=42, hop=writer-to-translator](\"hello\")",
    "return translator → host example:notes/decoration@0.1.0.decorate [session=42, hop=writer-to-translator](\"host[session=42, hop=writer-to-translator]: hello\")",
    "invocation end host",
    "return writer → translator example:notes/translator@0.1.0.translate [session=42, hop=writer-to-translator](\"host[session=42, hop=writer-to-translator]: hello #1\")",
    "invocation end translator",
    "return host → writer example:notes/writer@0.1.0.write [session=42](\"host[session=42, hop=writer-to-translator]: hello #1\")",
    "invocation end writer",
    "invocation start writer",
    "call host → writer example:notes/writer@0.1.0.write-async [session=42](\"async\")",
    "invocation start translator",
    "call writer → translator example:notes/translator@0.1.0.translate-async [session=42, hop=writer-to-translator](\"async\")",
    "invocation start host",
    "call translator → host example:notes/decoration@0.1.0.decorate-async [session=42, hop=writer-to-translator](\"async\")",
    "return translator → host example:notes/decoration@0.1.0.decorate-async [session=42, hop=writer-to-translator](\"host[session=42, hop=writer-to-translator]: async\")",
    "invocation end host",
    "return writer → translator example:notes/translator@0.1.0.translate-async [session=42, hop=writer-to-translator](\"host[session=42, hop=writer-to-translator]: async #1\")",
    "invocation end translator",
    "return host → writer example:notes/writer@0.1.0.write-async [session=42](\"host[session=42, hop=writer-to-translator]: async #1\")",
    "invocation end writer",
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
