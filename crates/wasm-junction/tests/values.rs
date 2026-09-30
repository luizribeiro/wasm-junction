//! Round-trip tests for plain WIT values.

use wasm_junction::{TypeError, Val};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Note {
    title: String,
    lines: Vec<String>,
}

impl From<Note> for Val {
    fn from(note: Note) -> Self {
        Self::Record(vec![
            ("title", note.title.into()),
            (
                "lines",
                Self::List(note.lines.into_iter().map(Into::into).collect()),
            ),
        ])
    }
}

impl TryFrom<Val> for Note {
    type Error = TypeError;

    fn try_from(value: Val) -> Result<Self, Self::Error> {
        let Val::Record(fields) = value else {
            return Err(TypeError::new("expected note record"));
        };
        let mut title = None;
        let mut lines = None;
        for (field, value) in fields {
            match (field, value) {
                ("title", value) => title = Some(value.try_into()?),
                ("lines", Val::List(values)) => {
                    lines = Some(
                        values
                            .into_iter()
                            .map(String::try_from)
                            .collect::<Result<_, _>>()?,
                    );
                }
                (field, _) => return Err(TypeError::new(format!("unknown field `{field}`"))),
            }
        }
        Ok(Self {
            title: title.ok_or_else(|| TypeError::new("missing field `title`"))?,
            lines: lines.ok_or_else(|| TypeError::new("missing field `lines`"))?,
        })
    }
}

fn pair_to_val(pair: (u32, String)) -> Val {
    Val::Tuple(vec![pair.0.into(), pair.1.into()])
}

fn pair_from_val(value: Val) -> Result<(u32, String), TypeError> {
    let Val::Tuple(mut values) = value else {
        return Err(TypeError::new("expected pair tuple"));
    };
    if values.len() != 2 {
        return Err(TypeError::new("pair tuple has the wrong arity"));
    }
    let text = values.pop().ok_or_else(|| TypeError::new("missing text"))?;
    let number = values
        .pop()
        .ok_or_else(|| TypeError::new("missing number"))?;
    Ok((number.try_into()?, text.try_into()?))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    Text(String),
    Line(u32),
    All,
}

impl From<Selection> for Val {
    fn from(selection: Selection) -> Self {
        match selection {
            Selection::Text(text) => Self::Variant {
                case: "text",
                value: Some(Box::new(text.into())),
            },
            Selection::Line(line) => Self::Variant {
                case: "line",
                value: Some(Box::new(line.into())),
            },
            Selection::All => Self::Variant {
                case: "all",
                value: None,
            },
        }
    }
}

impl TryFrom<Val> for Selection {
    type Error = TypeError;

    fn try_from(value: Val) -> Result<Self, Self::Error> {
        match value {
            Val::Variant {
                case: "text",
                value: Some(value),
            } => Ok(Self::Text((*value).try_into()?)),
            Val::Variant {
                case: "line",
                value: Some(value),
            } => Ok(Self::Line((*value).try_into()?)),
            Val::Variant {
                case: "all",
                value: None,
            } => Ok(Self::All),
            Val::Variant { case, .. } => Err(TypeError::new(format!("unknown case `{case}`"))),
            _ => Err(TypeError::new("expected selection variant")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Plain,
    Markdown,
}

impl From<Format> for Val {
    fn from(format: Format) -> Self {
        Self::Enum(match format {
            Format::Plain => "plain",
            Format::Markdown => "markdown",
        })
    }
}

impl TryFrom<Val> for Format {
    type Error = TypeError;

    fn try_from(value: Val) -> Result<Self, Self::Error> {
        match value {
            Val::Enum("plain") => Ok(Self::Plain),
            Val::Enum("markdown") => Ok(Self::Markdown),
            Val::Enum(case) => Err(TypeError::new(format!("unknown case `{case}`"))),
            _ => Err(TypeError::new("expected format enum")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Permissions {
    read: bool,
    summarize: bool,
}

impl From<Permissions> for Val {
    fn from(permissions: Permissions) -> Self {
        let mut flags = Vec::new();
        if permissions.read {
            flags.push("read");
        }
        if permissions.summarize {
            flags.push("summarize");
        }
        Self::Flags(flags)
    }
}

impl TryFrom<Val> for Permissions {
    type Error = TypeError;

    fn try_from(value: Val) -> Result<Self, Self::Error> {
        let Val::Flags(flags) = value else {
            return Err(TypeError::new("expected permissions flags"));
        };
        let mut permissions = Self {
            read: false,
            summarize: false,
        };
        for flag in flags {
            match flag {
                "read" => permissions.read = true,
                "summarize" => permissions.summarize = true,
                flag => return Err(TypeError::new(format!("unknown flag `{flag}`"))),
            }
        }
        Ok(permissions)
    }
}

#[test]
fn primitive_shapes_round_trip() {
    macro_rules! round_trip {
        ($value:expr, $type:ty) => {{
            assert_eq!(<$type>::try_from(Val::from($value)).unwrap(), $value);
        }};
    }

    round_trip!(true, bool);
    round_trip!(-8_i8, i8);
    round_trip!(8_u8, u8);
    round_trip!(-16_i16, i16);
    round_trip!(16_u16, u16);
    round_trip!(-32_i32, i32);
    round_trip!(32_u32, u32);
    round_trip!(-64_i64, i64);
    round_trip!(64_u64, u64);
    assert!((f32::try_from(Val::from(3.25_f32)).unwrap() - 3.25).abs() < f32::EPSILON);
    assert!((f64::try_from(Val::from(6.5_f64)).unwrap() - 6.5).abs() < f64::EPSILON);
    round_trip!('j', char);
    round_trip!(String::from("journal"), String);
    assert_eq!(Val::from("note"), Val::String(String::from("note")));
}

#[test]
fn primitive_shape_mismatches_are_type_errors() {
    let error: TypeError = u32::try_from(Val::Bool(true)).unwrap_err();
    assert_eq!(error.to_string(), "expected u32");
}

#[test]
fn list_record_and_tuple_shapes_round_trip() {
    let note = Note {
        title: String::from("Groceries"),
        lines: vec![String::from("tea"), String::from("oranges")],
    };
    assert_eq!(Note::try_from(Val::from(note.clone())).unwrap(), note);

    let pair = (7, String::from("weekly"));
    assert_eq!(pair_from_val(pair_to_val(pair.clone())).unwrap(), pair);
}

#[test]
fn record_fields_and_tuple_arity_are_checked() {
    assert!(pair_from_val(Val::Tuple(vec![1_u32.into()])).is_err());
    assert!(Note::try_from(Val::Record(vec![("subject", "news".into())])).is_err());
    assert!(Note::try_from(Val::Record(vec![("title", "News".into())])).is_err());
}

#[test]
fn every_variant_and_enum_case_round_trips() {
    for selection in [
        Selection::Text(String::from("summary")),
        Selection::Line(4),
        Selection::All,
    ] {
        assert_eq!(
            Selection::try_from(Val::from(selection.clone())).unwrap(),
            selection
        );
    }
    for format in [Format::Plain, Format::Markdown] {
        assert_eq!(Format::try_from(Val::from(format)).unwrap(), format);
    }
}

#[test]
fn enum_and_variant_cases_are_checked() {
    assert!(Format::try_from(Val::Enum("html")).is_err());
    assert!(
        Selection::try_from(Val::Variant {
            case: "range",
            value: None
        })
        .is_err()
    );
}

#[test]
fn flags_option_and_result_shapes_round_trip() {
    for permissions in [
        Permissions {
            read: true,
            summarize: true,
        },
        Permissions {
            read: false,
            summarize: false,
        },
    ] {
        assert_eq!(
            Permissions::try_from(Val::from(permissions)).unwrap(),
            permissions
        );
    }

    for option in [Some(String::from("draft")), None] {
        let value = Val::Option(option.clone().map(|value| Box::new(value.into())));
        let Val::Option(value) = value else {
            unreachable!();
        };
        let decoded = value.map(|value| String::try_from(*value).unwrap());
        assert_eq!(decoded, option);
    }

    let outcomes = [Ok(3_u32), Err(String::from("not found"))];
    for outcome in outcomes {
        let value = Val::Result(match outcome.clone() {
            Ok(value) => Ok(Some(Box::new(value.into()))),
            Err(error) => Err(Some(Box::new(error.into()))),
        });
        let Val::Result(value) = value else {
            unreachable!();
        };
        let decoded = match value {
            Ok(Some(value)) => Ok(u32::try_from(*value).unwrap()),
            Err(Some(error)) => Err(String::try_from(*error).unwrap()),
            _ => unreachable!(),
        };
        assert_eq!(decoded, outcome);
    }
}

#[test]
fn flag_names_are_checked() {
    assert!(Permissions::try_from(Val::Flags(vec!["delete"])).is_err());
}
