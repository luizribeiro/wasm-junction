//! Call tracing used by this example.

use wasm_junction::{Call, CallError, Middleware, Next, Val, Vals};

use crate::Output;

/// Middleware that prints every component boundary.
pub struct Trace(Output);

impl Trace {
    pub(crate) fn new(output: Output) -> Self {
        Self(output)
    }
}

impl Middleware for Trace {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let label = call.to_string();
        self.0(format!("call {label}({})", vals(&call.args)));
        let result = next.run(call).await;
        match &result {
            Ok(values) => self.0(format!("return {label}({})", vals(values))),
            Err(error) => self.0(format!("trap {label}({error})")),
        }
        result
    }
}

fn vals(values: &[Val]) -> String {
    values.iter().map(val).collect::<Vec<_>>().join(", ")
}

fn val(value: &Val) -> String {
    match value {
        Val::String(value) => format!("{value:?}"),
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
