//! Call and lifecycle tracing used by this example.

use std::collections::HashMap;
use std::sync::Mutex;

use wasm_junction::{
    Call, CallError, ChannelDirection, Event, Middleware, Next, ResourceOwnership, Val, Vals,
};

use crate::Output;

/// Middleware that prints calls, resource drops, and stream lifecycles.
pub struct Trace {
    output: Output,
    streams: Mutex<HashMap<u64, usize>>,
}

impl Trace {
    pub(crate) fn new(output: Output) -> Self {
        Self {
            output,
            streams: Mutex::default(),
        }
    }
}

impl Middleware for Trace {
    async fn call(&self, call: Call, next: Next) -> Result<Vals, CallError> {
        let label = call.to_string();
        (self.output)(format!("call {label}({})", vals(&call.args)));
        let result = next.run(call).await;
        match &result {
            Ok(values) => (self.output)(format!("return {label}({})", vals(values))),
            Err(error) => (self.output)(format!("trap {label}({error})")),
        }
        result
    }

    fn event(&self, event: &Event) {
        match event {
            Event::ResourceDrop {
                interface,
                resource,
                id,
            } => (self.output)(format!("resource drop {interface}/{resource}#{id}")),
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

impl Trace {
    fn channel(&self, action: &str, stream: u64, direction: ChannelDirection) {
        let mut streams = self.streams.lock().unwrap();
        let next = streams.len();
        let label = *streams.entry(stream).or_insert(next);
        let direction = match direction {
            ChannelDirection::HostToGuest => "host-to-guest",
            ChannelDirection::GuestToHost => "guest-to-host",
        };
        (self.output)(format!("channel {action} stream#{label} {direction}"));
    }
}

fn vals(values: &[Val]) -> String {
    values.iter().map(val).collect::<Vec<_>>().join(", ")
}

fn val(value: &Val) -> String {
    match value {
        Val::String(value) => format!("{value:?}"),
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
