use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use wasm_junction::{
    Call, CallError, ChannelDirection, Event, Middleware, Next, ResourceOwnership, Val, Vals,
};

use crate::{SessionId, TranslatorHop};

#[derive(Default)]
struct TraceState {
    entries: Vec<String>,
    streams: HashMap<u64, usize>,
}

/// Middleware that records calls, returns, traps, and invocation boundaries.
#[derive(Clone, Default)]
pub struct Trace(Arc<Mutex<TraceState>>, bool);

impl Trace {
    /// Creates a tracer that also records component load, reload, and unload events.
    #[must_use]
    pub fn with_lifecycle() -> Self {
        Self(Arc::default(), true)
    }

    /// Returns a snapshot of the recorded entries.
    #[must_use]
    pub fn entries(&self) -> Vec<String> {
        self.lock().entries.clone()
    }

    /// Removes all recorded entries.
    pub fn clear(&self) {
        *self.lock() = TraceState::default();
    }

    fn record(&self, entry: String) {
        self.lock().entries.push(entry);
    }

    fn lock(&self) -> MutexGuard<'_, TraceState> {
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
            Event::Load { component, exports } if self.1 => {
                self.record(format!("load {component} [{}]", interfaces(exports)));
            }
            Event::Reload {
                component,
                old_exports,
                new_exports,
            } if self.1 => self.record(format!(
                "reload {component} [{}] -> [{}]",
                interfaces(old_exports),
                interfaces(new_exports)
            )),
            Event::Unload { component } if self.1 => {
                self.record(format!("unload {component}"));
            }
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
            Event::ChannelOpen { stream, direction } => {
                self.channel("open", *stream, *direction);
            }
            Event::ChannelClose { stream, direction } => {
                self.channel("close", *stream, *direction);
            }
            _ => {}
        }
    }
}

fn interfaces(values: &[Arc<str>]) -> String {
    values
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join(", ")
}

impl Trace {
    fn channel(&self, action: &str, stream: u64, direction: ChannelDirection) {
        let mut state = self.lock();
        let next = state.streams.len();
        let label = *state.streams.entry(stream).or_insert(next);
        let direction = match direction {
            ChannelDirection::HostToGuest => "host-to-guest",
            ChannelDirection::GuestToHost => "guest-to-host",
        };
        state
            .entries
            .push(format!("channel {action} stream#{label} {direction}"));
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
        Val::Resource(resource) => format!(
            "{}({}/{}#{})",
            match resource.ownership() {
                ResourceOwnership::Own => "own",
                ResourceOwnership::Borrow => "borrow",
            },
            resource.interface(),
            resource.name(),
            resource.id()
        ),
        Val::Stream(_) => "stream".to_owned(),
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

/// Exact trace produced when the resource guest opens, uses, and drops a host session.
pub const EXPECTED_RESOURCE_TRACE: &[&str] = &[
    "invocation start resource-client",
    "call host → resource-client example:resources/client@1.0.0.run(false)",
    "invocation start host",
    "call resource-client → host example:resources/host@1.0.0.[constructor]session(\"Ada\")",
    "return resource-client → host example:resources/host@1.0.0.[constructor]session(own(example:resources/host@1.0.0/session#0))",
    "invocation end host",
    "invocation start host",
    "call resource-client → host example:resources/host@1.0.0.[method]session.profile(borrow(example:resources/host@1.0.0/session#0))",
    "return resource-client → host example:resources/host@1.0.0.[method]session.profile(\"profile:Ada\")",
    "invocation end host",
    "resource drop example:resources/host@1.0.0/session#0",
    "return host → resource-client example:resources/client@1.0.0.run(\"profile:Ada\")",
    "invocation end resource-client",
];

/// Exact trace produced by the host-to-guest and guest-to-host stream scenarios.
pub const EXPECTED_STREAM_TRACE: &[&str] = &[
    "invocation start streams",
    "call host → streams example:streams/probe@0.1.0.motd()",
    "invocation start host",
    "call streams → host example:streams/host@0.1.0.motd()",
    "return streams → host example:streams/host@0.1.0.motd(stream)",
    "invocation end host",
    "channel open stream#0 host-to-guest",
    "channel close stream#0 host-to-guest",
    "return host → streams example:streams/probe@0.1.0.motd(\"Have a good day.\")",
    "invocation end streams",
    "invocation start streams",
    "call host → streams example:streams/probe@0.1.0.audit()",
    "channel open stream#1 guest-to-host",
    "invocation start host",
    "call streams → host example:streams/host@0.1.0.audit(stream)",
    "channel close stream#1 guest-to-host",
    "return streams → host example:streams/host@0.1.0.audit()",
    "invocation end host",
    "return host → streams example:streams/probe@0.1.0.audit()",
    "invocation end streams",
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
        assert_eq!(EXPECTED_RESOURCE_TRACE.len(), 13);
    }

    #[test]
    fn channel_trace_correlates_opaque_ids_with_stable_labels() {
        let trace = Trace::default();
        trace.event(&Event::ChannelOpen {
            stream: 91,
            direction: ChannelDirection::HostToGuest,
        });
        trace.event(&Event::ChannelClose {
            stream: 91,
            direction: ChannelDirection::HostToGuest,
        });
        assert_eq!(
            trace.entries(),
            [
                "channel open stream#0 host-to-guest",
                "channel close stream#0 host-to-guest",
            ]
        );
    }
}
