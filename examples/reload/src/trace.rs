//! Call and component lifecycle tracing used by this example.

use std::sync::Arc;

use wasm_junction::{Call, CallError, Event, Middleware, Next, Val, Vals};

/// Middleware that prints every component boundary and lifecycle change.
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

    fn event(&self, event: &Event) {
        match event {
            Event::Load { component, exports } => {
                println!("load {component} [{}]", interfaces(exports));
            }
            Event::Reload {
                component,
                old_exports,
                new_exports,
            } => println!(
                "reload {component} [{}] -> [{}]",
                interfaces(old_exports),
                interfaces(new_exports)
            ),
            Event::Unload { component } => println!("unload {component}"),
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
