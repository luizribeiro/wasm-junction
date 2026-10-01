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
