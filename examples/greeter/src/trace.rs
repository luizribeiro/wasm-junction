//! Call tracing used by this example.

use wasm_junction::{Call, CallError, Middleware, Next, Val, Vals};

/// Middleware that prints calls and their outcomes.
pub struct Trace;

impl Middleware for Trace {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let label = call.to_string();
        println!("call {label}({})", vals(&call.args));
        let result = next.run(call).await;
        match &result {
            Ok(values) => println!("return {label}({})", vals(values)),
            Err(error) => println!("trap {label}({error})"),
        }
        result
    }
}

fn vals(values: &[Val]) -> String {
    values.iter().map(val).collect::<Vec<_>>().join(", ")
}

fn val(value: &Val) -> String {
    match value {
        Val::U32(value) => value.to_string(),
        Val::String(value) => format!("{value:?}"),
        Val::Record(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(name, value)| format!("{name}: {}", val(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
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
