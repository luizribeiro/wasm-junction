use js_sys::{Array, BigInt};
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
        (Val::String(value), ValueType::String) => JsValue::from_str(&value),
        (_, ValueType::Unsupported(name)) => return Err(unsupported(name)),
        (value, expected) => {
            return Err(CallError::trap(format!(
                "expected {}, got {value:?}",
                expected.name(),
            )));
        }
    };
    Ok(value)
}

fn lift(value: JsValue, expected: &ValueType) -> Result<Val, CallError> {
    macro_rules! number {
        ($ty:ty, $variant:ident) => {
            number(&value, expected)
                .and_then(integer::<$ty>)
                .map(Val::$variant)
        };
    }
    match expected {
        ValueType::Bool => value
            .as_bool()
            .map(Val::Bool)
            .ok_or_else(|| wrong_js_type(expected)),
        ValueType::S8 => number!(i8, S8),
        ValueType::U8 => number!(u8, U8),
        ValueType::S16 => number!(i16, S16),
        ValueType::U16 => number!(u16, U16),
        ValueType::S32 => number!(i32, S32),
        ValueType::U32 => number!(u32, U32),
        ValueType::S64 => bigint::<i64>(value, expected).map(Val::S64),
        ValueType::U64 => bigint::<u64>(value, expected).map(Val::U64),
        ValueType::String => value
            .as_string()
            .map(Val::String)
            .ok_or_else(|| wrong_js_type(expected)),
        ValueType::Unsupported(name) => Err(unsupported(name)),
    }
}

fn number(value: &JsValue, expected: &ValueType) -> Result<f64, CallError> {
    value.as_f64().ok_or_else(|| wrong_js_type(expected))
}

#[allow(clippy::cast_possible_truncation)]
fn integer<T>(value: f64) -> Result<T, CallError>
where
    T: TryFrom<i64>,
{
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(CallError::trap("expected an integer from JavaScript"));
    }
    T::try_from(value as i64).map_err(|_| CallError::trap("JavaScript integer is out of range"))
}

fn bigint<T>(value: JsValue, expected: &ValueType) -> Result<T, CallError>
where
    T: TryFrom<BigInt>,
{
    value
        .dyn_into::<BigInt>()
        .map_err(|_| wrong_js_type(expected))
        .and_then(|value| {
            T::try_from(value).map_err(|_| {
                CallError::trap(format!("JavaScript {} is out of range", expected.name()))
            })
        })
}

fn wrong_js_type(expected: &ValueType) -> CallError {
    CallError::trap(format!("expected JavaScript {}", expected.name()))
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
    fn round_trips_integers_booleans_and_strings() {
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
    fn refuses_an_out_of_range_u8() {
        assert_eq!(
            lift_error(JsValue::from_f64(300.0), ValueType::U8),
            "JavaScript integer is out of range"
        );
    }

    #[wasm_bindgen_test]
    fn refuses_a_fractional_s32() {
        assert_eq!(
            lift_error(JsValue::from_f64(3.5), ValueType::S32),
            "expected an integer from JavaScript"
        );
    }

    #[wasm_bindgen_test]
    fn refuses_out_of_range_bigints() {
        assert_eq!(
            lift_error(BigInt::from(u128::MAX).into(), ValueType::U64),
            "JavaScript u64 is out of range"
        );
        assert_eq!(
            lift_error(BigInt::from(u64::MAX).into(), ValueType::S64),
            "JavaScript s64 is out of range"
        );
    }

    #[wasm_bindgen_test]
    fn refuses_a_string_where_a_number_is_expected() {
        assert_eq!(
            lift_error(JsValue::from_str("three"), ValueType::U32),
            "expected JavaScript u32"
        );
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
