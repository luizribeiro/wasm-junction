//! Call and component lifecycle tracing used by this example.

use std::sync::Arc;

use wasm_junction::{Call, CallError, Event, Middleware, Next, Val, Vals};

use crate::Output;

/// Middleware that prints every component boundary and lifecycle change.
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

    fn event(&self, event: &Event) {
        match event {
            Event::Load { component, exports } => {
                self.0(format!("load {component} [{}]", interfaces(exports)));
            }
            Event::Reload {
                component,
                old_exports,
                new_exports,
            } => self.0(format!(
                "reload {component} [{}] -> [{}]",
                interfaces(old_exports),
                interfaces(new_exports)
            )),
            Event::Unload { component } => self.0(format!("unload {component}")),
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

fn vals(values: &[Val]) -> String {
    values.iter().map(val).collect::<Vec<_>>().join(", ")
}

fn val(value: &Val) -> String {
    let value: &dyn std::fmt::Debug = match value {
        Val::String(value) => value,
        value => value,
    };
    format!("{value:?}")
}
