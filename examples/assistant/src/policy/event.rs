use std::path::Path;

use wasm_junction::{Call, Val};

use super::Event;

const FILESYSTEM: &str = "wasi:filesystem/types@0.3.0";

impl Event {
    pub(super) fn from_call(call: &Call, base: &Path) -> Result<Self, String> {
        let mut arguments = call.args.iter().map(value_text).collect::<Vec<_>>();
        if call.interface.as_ref() == FILESYSTEM
            && call.function.as_ref() == "[method]descriptor.open-at"
        {
            let path = string_arg(call.args.get(2), "filesystem path")?;
            let root = string_arg(call.args.last(), "filesystem preopen")?;
            arguments[2] = crate::path::resolve(base, root, path)?;
        }
        Ok(Self {
            caller: call.caller.to_string(),
            callee: call.callee.to_string(),
            interface: call.interface.to_string(),
            function: call.function.to_string(),
            arguments,
        })
    }

    pub(super) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "caller": self.caller,
            "callee": self.callee,
            "interface": self.interface,
            "function": self.function,
            "arguments": self.arguments,
        })
    }
}

fn string_arg<'a>(value: Option<&'a Val>, name: &str) -> Result<&'a str, String> {
    match value {
        Some(Val::String(value)) => Ok(value),
        _ => Err(format!("{name} is not text")),
    }
}

pub(super) fn value_text(value: &Val) -> String {
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
        Val::Char(value) => value.to_string(),
        Val::String(value) | Val::Enum(value) => value.clone(),
        Val::Bytes(value) => format!("<{} bytes>", value.len()),
        Val::List(values) => sequence("[", "]", values),
        Val::Tuple(values) => sequence("(", ")", values),
        Val::Record(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(name, value)| format!("{name}: {}", value_text(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Val::Variant { case, value } => value.as_deref().map_or_else(
            || case.clone(),
            |value| format!("{case}({})", value_text(value)),
        ),
        Val::Flags(values) => format!("{{{}}}", values.join(", ")),
        Val::Option(value) => value.as_deref().map_or_else(|| "none".into(), value_text),
        Val::Result(value) => match value {
            Ok(value) => tagged("ok", value.as_deref()),
            Err(value) => tagged("error", value.as_deref()),
        },
        Val::Resource(_) => "<resource>".into(),
        Val::Stream(_) => "<stream>".into(),
        Val::Future(_) => "<future>".into(),
        _ => "<value>".into(),
    }
}

fn sequence(open: &str, close: &str, values: &[Val]) -> String {
    format!(
        "{open}{}{close}",
        values.iter().map(value_text).collect::<Vec<_>>().join(", ")
    )
}

fn tagged(name: &str, value: Option<&Val>) -> String {
    value.map_or_else(
        || name.into(),
        |value| format!("{name}({})", value_text(value)),
    )
}
