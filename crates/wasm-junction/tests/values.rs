//! Round-trip tests for plain WIT values.

use wasm_junction::{TypeError, Val};

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
