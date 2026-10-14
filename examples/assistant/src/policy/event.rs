use std::path::Path;

use wasm_junction::{Call, Val};

use super::Event;

const FILESYSTEM: &str = "wasi:filesystem/types@0.3.0";

impl Event {
    pub(super) fn from_call(call: &Call, base: &Path) -> Result<Self, String> {
        let mut arguments = call.args.iter().map(value_text).collect::<Vec<_>>();
        if call.interface.as_ref() == FILESYSTEM {
            let paths: &[(usize, usize)] = match call.function.as_ref() {
                "[method]descriptor.open-at"
                | "[method]descriptor.stat-at"
                | "[method]descriptor.metadata-hash-at"
                | "[method]descriptor.set-times-at"
                | "[method]descriptor.symlink-at" => &[(2, 1)],
                "[method]descriptor.readlink-at"
                | "[method]descriptor.create-directory-at"
                | "[method]descriptor.remove-directory-at"
                | "[method]descriptor.unlink-file-at" => &[(1, 1)],
                "[method]descriptor.rename-at" => &[(1, 2), (3, 1)],
                "[method]descriptor.link-at" => &[(2, 2), (4, 1)],
                _ => &[],
            };
            for &(path_position, directory_offset) in paths {
                resolve_path(
                    &mut arguments,
                    call,
                    base,
                    path_position,
                    from_end(call, directory_offset)?,
                )?;
            }
            if matches!(
                call.function.as_ref(),
                "[method]descriptor.set-size" | "[method]descriptor.set-times"
            ) {
                let position = from_end(call, 1)?;
                let guest_path = string_arg(call.args.get(position), "filesystem guest path")?;
                let argument = arguments
                    .get_mut(position)
                    .ok_or_else(|| "filesystem guest path is missing".to_owned())?;
                *argument = crate::path::resolve(base, guest_path, ".")?;
            }
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

fn from_end(call: &Call, offset: usize) -> Result<usize, String> {
    call.args
        .len()
        .checked_sub(offset)
        .ok_or_else(|| "filesystem guest path is missing".to_owned())
}

fn resolve_path(
    arguments: &mut [String],
    call: &Call,
    base: &Path,
    path_position: usize,
    directory_position: usize,
) -> Result<(), String> {
    let path = string_arg(call.args.get(path_position), "filesystem path")?;
    let directory = string_arg(
        call.args.get(directory_position),
        "filesystem guest directory",
    )?;
    let argument = arguments
        .get_mut(path_position)
        .ok_or_else(|| "filesystem path is missing".to_owned())?;
    *argument = crate::path::resolve(base, directory, path)?;
    Ok(())
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
