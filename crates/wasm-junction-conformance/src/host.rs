use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use wasm_junction::{BoxFuture, Call, CallContext, Provided, Provider, Trap, Val, Vals};

use crate::NOTES;

/// The host implementation used by the notes-summary fixture.
#[derive(Clone, Default)]
pub struct FixtureHost {
    normalizations: Arc<AtomicUsize>,
}

impl FixtureHost {
    /// Wraps this host as the fixture's notes provider.
    #[must_use]
    pub fn provided(self) -> Provided {
        Provided::new(NOTES, self)
    }

    /// Returns the number of notes normalized by this host.
    #[must_use]
    pub fn normalizations(&self) -> usize {
        self.normalizations.load(Ordering::Relaxed)
    }
}

impl Provider for FixtureHost {
    fn call<'a>(
        &'a self,
        _context: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, Trap>> {
        Box::pin(async move {
            match call.function.as_ref() {
                "read" => read(&call.args),
                "normalize" => {
                    self.normalizations.fetch_add(1, Ordering::Relaxed);
                    normalize(call.args)
                }
                function => Err(Trap::new(format!("unknown notes function `{function}`"))),
            }
        })
    }
}

fn read(args: &[Val]) -> Result<Vals, Trap> {
    let [Val::String(name)] = args else {
        return Err(Trap::new("notes.read expected one string"));
    };
    let result = if name == "private" {
        Err(Some(Box::new(Val::String("permission denied".to_owned()))))
    } else {
        Ok(Some(Box::new(sample_note())))
    };
    Ok(vec![Val::Result(result)])
}

fn normalize(args: Vals) -> Result<Vals, Trap> {
    if args.len() != 1 || !matches!(args.first(), Some(Val::Record(_))) {
        return Err(Trap::new("notes.normalize expected one note"));
    }
    Ok(args)
}

/// Returns the fixture note, containing every plain WIT value shape.
#[must_use]
pub fn sample_note() -> Val {
    Val::Record(vec![
        field("title", Val::String("Daily".to_owned())),
        field("published", Val::Bool(true)),
        field("signed-8", Val::S8(-8)),
        field("unsigned-8", Val::U8(8)),
        field("signed-16", Val::S16(-16)),
        field("unsigned-16", Val::U16(16)),
        field("signed-32", Val::S32(-32)),
        field("unsigned-32", Val::U32(32)),
        field("signed-64", Val::S64(-64)),
        field("unsigned-64", Val::U64(64)),
        field("score-32", Val::F32(3.5)),
        field("score-64", Val::F64(7.25)),
        field("marker", Val::Char('§')),
        field(
            "tags",
            Val::List(vec![Val::from("rust"), Val::from("wasm")]),
        ),
        field("location", Val::Tuple(vec![Val::S32(-71), Val::S32(42)])),
        field(
            "attachment",
            Val::Variant {
                case: "text".to_owned(),
                value: Some(Box::new(Val::from("diagram"))),
            },
        ),
        field("mood", Val::Enum("upbeat".to_owned())),
        field(
            "emphasis",
            Val::Flags(vec!["concise".to_owned(), "detailed".to_owned()]),
        ),
        field(
            "subtitle",
            Val::Option(Some(Box::new(Val::from("Engine notes")))),
        ),
        field("revision", Val::Result(Ok(Some(Box::new(Val::U64(7)))))),
    ])
}

/// Returns the fixture's successful summary result.
#[must_use]
pub fn sample_summary() -> Val {
    Val::Result(Ok(Some(Box::new(Val::Record(vec![
        field("text", Val::from("Daily: 2 tags")),
        field("source", sample_note()),
    ])))))
}

fn field(name: &str, value: Val) -> (String, Val) {
    (name.to_owned(), value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_is_a_typed_result() {
        let result = read(&[Val::from("private")]).unwrap();
        assert_eq!(
            result,
            [Val::Result(Err(Some(Box::new(Val::from(
                "permission denied"
            )))))]
        );
    }
}
