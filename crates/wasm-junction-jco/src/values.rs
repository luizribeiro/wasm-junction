use js_sys::{Array, BigInt, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_junction_core::{CallError, Val, Vals};

use crate::types::{FunctionType, ValueType};

pub(crate) fn lower_args(values: Vals, signature: &FunctionType) -> Result<Array, CallError> {
    if values.len() != signature.params.len() {
        return Err(CallError::trap("component argument count mismatch"));
    }
    values
        .into_iter()
        .zip(&signature.params)
        .map(|(value, expected)| lower(value, expected))
        .collect()
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn lift_args(values: &Array, signature: &FunctionType) -> Result<Vals, CallError> {
    if values.length() as usize != signature.params.len() {
        return Err(CallError::trap("imported argument count mismatch"));
    }
    signature
        .params
        .iter()
        .enumerate()
        .map(|(index, ty)| lift(values.get(index as u32), ty))
        .collect()
}

pub(crate) fn lower_result(values: &Vals, signature: &FunctionType) -> Result<JsValue, CallError> {
    match (values.as_slice(), &signature.result) {
        ([], None) => Ok(JsValue::UNDEFINED),
        ([value], Some(ty)) => lower(value.clone(), ty),
        _ => Err(CallError::trap("imported result count mismatch")),
    }
}

pub(crate) fn lift_result(value: JsValue, signature: &FunctionType) -> Result<Vals, CallError> {
    signature.result.as_ref().map_or_else(
        || Ok(Vec::new()),
        |ty| lift(value, ty).map(|value| vec![value]),
    )
}

fn lower(value: Val, expected: &ValueType) -> Result<JsValue, CallError> {
    let value = match (value, expected) {
        (Val::Bool(value), ValueType::Bool) => JsValue::from_bool(value),
        (Val::S8(value), ValueType::S8) => JsValue::from_f64(f64::from(value)),
        (Val::U8(value), ValueType::U8) => JsValue::from_f64(f64::from(value)),
        (Val::S16(value), ValueType::S16) => JsValue::from_f64(f64::from(value)),
        (Val::U16(value), ValueType::U16) => JsValue::from_f64(f64::from(value)),
        (Val::S32(value), ValueType::S32) => JsValue::from_f64(f64::from(value)),
        (Val::U32(value), ValueType::U32) => JsValue::from_f64(f64::from(value)),
        (Val::S64(value), ValueType::S64) => BigInt::from(value).into(),
        (Val::U64(value), ValueType::U64) => BigInt::from(value).into(),
        (Val::F32(value), ValueType::F32) => JsValue::from_f64(f64::from(value)),
        (Val::F64(value), ValueType::F64) => JsValue::from_f64(value),
        (Val::Char(value), ValueType::Char) => JsValue::from_str(&value.to_string()),
        (Val::String(value), ValueType::String) => JsValue::from_str(&value),
        (Val::List(values), ValueType::List(element)) if **element == ValueType::U8 => {
            let bytes = values
                .into_iter()
                .map(|value| match value {
                    Val::U8(value) => Ok(value),
                    value => Err(wrong_val_type(element, &value)),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Uint8Array::from(bytes.as_slice()).into()
        }
        (Val::List(values), ValueType::List(element)) => lower_sequence(values, element)?,
        (Val::Tuple(values), ValueType::Tuple(elements)) => {
            if values.len() != elements.len() {
                return Err(mismatch(
                    expected,
                    &Val::Tuple(values),
                    "wrong element count",
                ));
            }
            values
                .into_iter()
                .zip(elements)
                .map(|(value, element)| lower(value, element))
                .collect::<Result<Array, _>>()?
                .into()
        }
        (_, ValueType::Unsupported(name)) => return Err(unsupported(name)),
        (_, ValueType::Record(_)) => return Err(unsupported("record")),
        (_, ValueType::Variant(_)) => return Err(unsupported("variant")),
        (_, ValueType::Enum(_)) => return Err(unsupported("enum")),
        (_, ValueType::Flags(_)) => return Err(unsupported("flags")),
        (_, ValueType::Option(_)) => return Err(unsupported("option")),
        (_, ValueType::Result { .. }) => return Err(unsupported("result")),
        (value, expected) => {
            return Err(wrong_val_type(expected, &value));
        }
    };
    Ok(value)
}

fn lift(value: JsValue, expected: &ValueType) -> Result<Val, CallError> {
    macro_rules! number {
        ($ty:ty, $variant:ident) => {
            number(&value, expected)
                .and_then(|value| integer::<$ty>(value, expected))
                .map(Val::$variant)
        };
    }
    match expected {
        ValueType::Bool => value
            .as_bool()
            .map(Val::Bool)
            .ok_or_else(|| wrong_js_type(expected, &value)),
        ValueType::S8 => number!(i8, S8),
        ValueType::U8 => number!(u8, U8),
        ValueType::S16 => number!(i16, S16),
        ValueType::U16 => number!(u16, U16),
        ValueType::S32 => number!(i32, S32),
        ValueType::U32 => number!(u32, U32),
        ValueType::S64 => bigint::<i64>(value, expected).map(Val::S64),
        ValueType::U64 => bigint::<u64>(value, expected).map(Val::U64),
        ValueType::F32 => number(&value, expected).map(|value| Val::F32(value as f32)),
        ValueType::F64 => number(&value, expected).map(Val::F64),
        ValueType::Char => value
            .as_string()
            .and_then(one_char)
            .map(Val::Char)
            .ok_or_else(|| mismatch(expected, &value, "expected one Unicode scalar value")),
        ValueType::String => value
            .as_string()
            .map(Val::String)
            .ok_or_else(|| wrong_js_type(expected, &value)),
        ValueType::List(element) if **element == ValueType::U8 => value
            .dyn_into::<Uint8Array>()
            .map(|values| Val::List(values.to_vec().into_iter().map(Val::U8).collect()))
            .map_err(|value| wrong_js_type(expected, &value)),
        ValueType::List(element) => lift_sequence(value, element).map(Val::List),
        ValueType::Tuple(elements) => {
            let values = js_array(value, expected)?;
            if values.length() as usize != elements.len() {
                return Err(mismatch(expected, &values, "wrong element count"));
            }
            elements
                .iter()
                .enumerate()
                .map(|(index, element)| lift(values.get(index as u32), element))
                .collect::<Result<Vec<_>, _>>()
                .map(Val::Tuple)
        }
        ValueType::Record(_) => Err(unsupported("record")),
        ValueType::Variant(_) => Err(unsupported("variant")),
        ValueType::Enum(_) => Err(unsupported("enum")),
        ValueType::Flags(_) => Err(unsupported("flags")),
        ValueType::Option(_) => Err(unsupported("option")),
        ValueType::Result { .. } => Err(unsupported("result")),
        ValueType::Unsupported(name) => Err(unsupported(name)),
    }
}

fn lower_sequence(values: Vec<Val>, element: &ValueType) -> Result<JsValue, CallError> {
    values
        .into_iter()
        .map(|value| lower(value, element))
        .collect::<Result<Array, _>>()
        .map(Into::into)
}

fn lift_sequence(value: JsValue, element: &ValueType) -> Result<Vec<Val>, CallError> {
    js_array(value, &ValueType::List(Box::new(element.clone())))?
        .iter()
        .map(|value| lift(value, element))
        .collect()
}

fn js_array(value: JsValue, expected: &ValueType) -> Result<Array, CallError> {
    if Array::is_array(&value) {
        Ok(Array::from(&value))
    } else {
        Err(wrong_js_type(expected, &value))
    }
}

fn one_char(value: String) -> Option<char> {
    let mut characters = value.chars();
    let character = characters.next()?;
    characters.next().is_none().then_some(character)
}

fn number(value: &JsValue, expected: &ValueType) -> Result<f64, CallError> {
    value.as_f64().ok_or_else(|| wrong_js_type(expected, value))
}

#[allow(clippy::cast_possible_truncation)]
fn integer<T>(value: f64, expected: &ValueType) -> Result<T, CallError>
where
    T: TryFrom<i64>,
{
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(mismatch(expected, &value, "expected an integer"));
    }
    T::try_from(value as i64).map_err(|_| mismatch(expected, &value, "integer is out of range"))
}

fn bigint<T>(value: JsValue, expected: &ValueType) -> Result<T, CallError>
where
    T: TryFrom<BigInt>,
{
    value
        .dyn_into::<BigInt>()
        .map_err(|value| wrong_js_type(expected, &value))
        .and_then(|value| {
            T::try_from(value.clone())
                .map_err(|_| mismatch(expected, &value, "integer is out of range"))
        })
}

fn wrong_val_type(expected: &ValueType, value: &Val) -> CallError {
    mismatch(expected, value, "wrong framework value kind")
}

fn wrong_js_type(expected: &ValueType, value: &JsValue) -> CallError {
    mismatch(expected, value, "wrong JavaScript value kind")
}

fn mismatch(expected: &ValueType, value: &impl std::fmt::Debug, reason: &str) -> CallError {
    CallError::trap(format!(
        "invalid WIT `{}` value {value:?}: {reason}",
        expected.name()
    ))
}

fn unsupported(name: &str) -> CallError {
    CallError::trap(format!("jco does not yet support WIT `{name}` values"))
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    use super::*;

    wasm_bindgen_test_configure!(run_in_dedicated_worker);

    #[wasm_bindgen_test]
    fn round_trips_primitive_values() {
        let (values, params): (Vals, Vec<ValueType>) = [
            (Val::Bool(true), ValueType::Bool),
            (Val::S8(-8), ValueType::S8),
            (Val::U8(8), ValueType::U8),
            (Val::S16(-16), ValueType::S16),
            (Val::U16(16), ValueType::U16),
            (Val::S32(-32), ValueType::S32),
            (Val::U32(32), ValueType::U32),
            (Val::S64(-64), ValueType::S64),
            (Val::U64(64), ValueType::U64),
            (Val::F32(3.5), ValueType::F32),
            (Val::F64(7.25), ValueType::F64),
            (Val::Char('§'), ValueType::Char),
            (Val::String("value".to_owned()), ValueType::String),
        ]
        .into_iter()
        .unzip();
        let signature = FunctionType {
            params,
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
        let result = vec![Val::String("result".to_owned())];
        let signature = FunctionType {
            params: Vec::new(),
            result: Some(ValueType::String),
        };
        let lowered = lower_result(&result, &signature).unwrap();
        assert_eq!(lift_result(lowered, &signature).unwrap(), result);
        let signature = FunctionType {
            params: vec![ValueType::Unsupported("record")],
            result: None,
        };
        let error = lower_args(vec![Val::Record(Vec::new())], &signature).unwrap_err();
        assert_eq!(
            error.to_string(),
            "jco does not yet support WIT `record` values"
        );
    }

    #[wasm_bindgen_test]
    fn round_trips_lists_byte_lists_and_tuples() {
        let values = vec![
            Val::List(vec![Val::from("rust"), Val::from("wasm")]),
            Val::List(vec![Val::U8(1), Val::U8(2)]),
            Val::Tuple(vec![Val::S32(-71), Val::S32(42)]),
        ];
        let signature = FunctionType {
            params: vec![
                ValueType::List(Box::new(ValueType::String)),
                ValueType::List(Box::new(ValueType::U8)),
                ValueType::Tuple(vec![ValueType::S32, ValueType::S32]),
            ],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert!(lowered.get(1).is_instance_of::<Uint8Array>());
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn refuses_an_out_of_range_u8() {
        let error = lift_error(JsValue::from_f64(300.0), ValueType::U8);
        assert!(error.contains("WIT `u8`"), "{error}");
        assert!(error.contains("300"), "{error}");
        assert!(error.contains("out of range"), "{error}");
    }

    #[wasm_bindgen_test]
    fn refuses_a_fractional_s32() {
        let error = lift_error(JsValue::from_f64(3.5), ValueType::S32);
        assert!(error.contains("WIT `s32`"), "{error}");
        assert!(error.contains("3.5"), "{error}");
        assert!(error.contains("expected an integer"), "{error}");
    }

    #[wasm_bindgen_test]
    fn refuses_out_of_range_bigints() {
        for (value, ty) in [
            (BigInt::from(u128::MAX).into(), ValueType::U64),
            (BigInt::from(u64::MAX).into(), ValueType::S64),
        ] {
            let error = lift_error(value, ty);
            assert!(error.contains("out of range"), "{error}");
        }
    }

    #[wasm_bindgen_test]
    fn refuses_a_string_where_a_number_is_expected() {
        let error = lift_error(JsValue::from_str("three"), ValueType::U32);
        assert!(error.contains("WIT `u32`"), "{error}");
        assert!(error.contains("three"), "{error}");
    }

    fn lift_error(value: JsValue, expected: ValueType) -> String {
        let arguments = Array::new();
        arguments.push(&value);
        lift_args(
            &arguments,
            &FunctionType {
                params: vec![expected],
                result: None,
            },
        )
        .unwrap_err()
        .to_string()
    }
}
