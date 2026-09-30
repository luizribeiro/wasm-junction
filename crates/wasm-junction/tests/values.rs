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
