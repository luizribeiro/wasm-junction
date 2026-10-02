use js_sys::{Array, BigInt, Object, Reflect, Uint8Array};
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
        (Val::Record(values), ValueType::Record(fields)) => lower_record(values, fields, expected)?,
        (Val::Variant { case, value }, ValueType::Variant(cases)) => {
            lower_variant(case, value, cases, expected)?
        }
        (Val::Enum(case), ValueType::Enum(cases)) if cases.contains(&case) => {
            JsValue::from_str(&case)
        }
        (Val::Flags(names), ValueType::Flags(flags)) => lower_flags(names, flags, expected)?,
        (Val::Option(value), ValueType::Option(payload)) => lower_option(value, payload, expected)?,
        (Val::Result(value), ValueType::Result { ok, err }) => {
            lower_nested_result(value, ok.as_deref(), err.as_deref(), expected)?
        }
        (_, ValueType::Unsupported(name)) => return Err(unsupported(name)),
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
        ValueType::Record(fields) => lift_record(value, fields, expected),
        ValueType::Variant(cases) => lift_variant(value, cases, expected),
        ValueType::Enum(cases) => value
            .as_string()
            .filter(|case| cases.contains(case))
            .map(Val::Enum)
            .ok_or_else(|| mismatch(expected, &value, "unknown enum case")),
        ValueType::Flags(flags) => lift_flags(value, flags, expected),
        ValueType::Option(payload) => lift_option(value, payload, expected),
        ValueType::Result { ok, err } => {
            lift_nested_result(value, ok.as_deref(), err.as_deref(), expected)
        }
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

fn lower_record(
    mut values: Vec<(String, Val)>,
    fields: &[crate::types::FieldType],
    expected: &ValueType,
) -> Result<JsValue, CallError> {
    for field in fields {
        if !values.iter().any(|(name, _)| name == &field.name) {
            return Err(mismatch(
                expected,
                &Val::Record(values),
                &format!("missing field `{}`", field.name),
            ));
        }
    }
    if values.len() != fields.len()
        || values
            .iter()
            .any(|(name, _)| !fields.iter().any(|field| &field.name == name))
    {
        return Err(mismatch(
            expected,
            &Val::Record(values),
            "unknown or duplicate field",
        ));
    }
    let object = Object::new();
    for field in fields {
        let index = values
            .iter()
            .position(|(name, _)| name == &field.name)
            .ok_or_else(|| mismatch(expected, &values, "record changed during conversion"))?;
        let (_, value) = values.remove(index);
        Reflect::set(
            &object,
            &JsValue::from_str(&field.js_name),
            &lower(value, &field.ty)?,
        )
        .map_err(|error| mismatch(expected, &error, "could not set record field"))?;
    }
    Ok(object.into())
}

fn lift_record(
    value: JsValue,
    fields: &[crate::types::FieldType],
    expected: &ValueType,
) -> Result<Val, CallError> {
    if !value.is_object() || Array::is_array(&value) {
        return Err(wrong_js_type(expected, &value));
    }
    let object = Object::from(value.clone());
    for key in Object::keys(&object)
        .iter()
        .filter_map(|key| key.as_string())
    {
        if !fields.iter().any(|field| field.js_name == key) {
            return Err(mismatch(
                expected,
                &value,
                &format!("unknown field `{key}`"),
            ));
        }
    }
    let values = fields
        .iter()
        .map(|field| {
            let key = JsValue::from_str(&field.js_name);
            let present = Reflect::has(&object, &key)
                .map_err(|error| mismatch(expected, &error, "could not inspect record field"))?;
            if !present && !matches!(field.ty, ValueType::Option(_)) {
                return Err(mismatch(
                    expected,
                    &value,
                    &format!("missing field `{}`", field.name),
                ));
            }
            Reflect::get(&object, &key)
                .map_err(|error| mismatch(expected, &error, "could not read record field"))
                .and_then(|value| lift(value, &field.ty))
                .map(|value| (field.name.clone(), value))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Val::Record(values))
}

fn lower_variant(
    case: String,
    value: Option<Box<Val>>,
    cases: &[crate::types::CaseType],
    expected: &ValueType,
) -> Result<JsValue, CallError> {
    let actual = Val::Variant {
        case: case.clone(),
        value: value.clone(),
    };
    let Some(case_type) = cases.iter().find(|candidate| candidate.name == case) else {
        return Err(mismatch(expected, &actual, "unknown variant case"));
    };
    let object = Object::new();
    Reflect::set(&object, &"tag".into(), &JsValue::from_str(&case))
        .map_err(|error| mismatch(expected, &error, "could not set variant tag"))?;
    match (value, &case_type.ty) {
        (None, None) => {}
        (Some(value), Some(ty)) => {
            Reflect::set(&object, &"val".into(), &lower(*value, ty)?)
                .map_err(|error| mismatch(expected, &error, "could not set variant payload"))?;
        }
        _ => {
            return Err(mismatch(
                expected,
                &actual,
                "wrong payload for variant case",
            ));
        }
    }
    Ok(object.into())
}

fn lift_variant(
    value: JsValue,
    cases: &[crate::types::CaseType],
    expected: &ValueType,
) -> Result<Val, CallError> {
    if !value.is_object() || Array::is_array(&value) {
        return Err(wrong_js_type(expected, &value));
    }
    let tag = Reflect::get(&value, &"tag".into())
        .ok()
        .and_then(|tag| tag.as_string())
        .ok_or_else(|| mismatch(expected, &value, "missing string tag"))?;
    let case = cases
        .iter()
        .find(|candidate| candidate.name == tag)
        .ok_or_else(|| mismatch(expected, &value, "unknown variant case"))?;
    let payload = case
        .ty
        .as_ref()
        .map(|ty| {
            Reflect::get(&value, &"val".into())
                .map_err(|error| mismatch(expected, &error, "could not read variant payload"))
                .and_then(|value| lift(value, ty))
                .map(Box::new)
        })
        .transpose()?;
    Ok(Val::Variant {
        case: tag,
        value: payload,
    })
}

fn lower_flags(
    names: Vec<String>,
    flags: &[String],
    expected: &ValueType,
) -> Result<JsValue, CallError> {
    if names.iter().any(|name| !flags.contains(name)) {
        return Err(mismatch(expected, &Val::Flags(names), "unknown flag"));
    }
    let object = Object::new();
    for flag in flags {
        Reflect::set(
            &object,
            &JsValue::from_str(&crate::types::js_name(flag)),
            &JsValue::from_bool(names.contains(flag)),
        )
        .map_err(|error| mismatch(expected, &error, "could not set flag"))?;
    }
    Ok(object.into())
}

fn lift_flags(value: JsValue, flags: &[String], expected: &ValueType) -> Result<Val, CallError> {
    if !value.is_object() || Array::is_array(&value) {
        return Err(wrong_js_type(expected, &value));
    }
    let object = Object::from(value.clone());
    for key in Object::keys(&object)
        .iter()
        .filter_map(|key| key.as_string())
    {
        if !flags.iter().any(|flag| crate::types::js_name(flag) == key) {
            return Err(mismatch(expected, &value, &format!("unknown flag `{key}`")));
        }
    }
    let mut names = Vec::new();
    for flag in flags {
        let key = JsValue::from_str(&crate::types::js_name(flag));
        if Reflect::has(&object, &key)
            .map_err(|error| mismatch(expected, &error, "could not inspect flag"))?
        {
            let active = Reflect::get(&object, &key)
                .map_err(|error| mismatch(expected, &error, "could not read flag"))?
                .as_bool()
                .ok_or_else(|| mismatch(expected, &value, "flag was not a boolean"))?;
            if active {
                names.push(flag.clone());
            }
        }
    }
    Ok(Val::Flags(names))
}

fn lower_option(
    value: Option<Box<Val>>,
    payload: &ValueType,
    expected: &ValueType,
) -> Result<JsValue, CallError> {
    match (value, maybe_null(payload)) {
        (None, false) => Ok(JsValue::UNDEFINED),
        (Some(value), false) => lower(*value, payload),
        (None, true) => tagged("none", None, expected),
        (Some(value), true) => tagged("some", Some(lower(*value, payload)?), expected),
    }
}

fn lift_option(
    value: JsValue,
    payload: &ValueType,
    expected: &ValueType,
) -> Result<Val, CallError> {
    if !maybe_null(payload) {
        return if value.is_null() || value.is_undefined() {
            Ok(Val::Option(None))
        } else {
            lift(value, payload).map(|value| Val::Option(Some(Box::new(value))))
        };
    }
    let (tag, value) = tagged_parts(&value, expected)?;
    match tag.as_str() {
        "none" => Ok(Val::Option(None)),
        "some" => lift(value, payload).map(|value| Val::Option(Some(Box::new(value)))),
        _ => Err(mismatch(expected, &value, "unknown option case")),
    }
}

fn maybe_null(ty: &ValueType) -> bool {
    matches!(ty, ValueType::Option(payload) if !maybe_null(payload))
}

fn tagged(tag: &str, value: Option<JsValue>, expected: &ValueType) -> Result<JsValue, CallError> {
    let object = Object::new();
    Reflect::set(&object, &"tag".into(), &JsValue::from_str(tag))
        .map_err(|error| mismatch(expected, &error, "could not set tag"))?;
    if let Some(value) = value {
        Reflect::set(&object, &"val".into(), &value)
            .map_err(|error| mismatch(expected, &error, "could not set payload"))?;
    }
    Ok(object.into())
}

fn tagged_parts(value: &JsValue, expected: &ValueType) -> Result<(String, JsValue), CallError> {
    if !value.is_object() || Array::is_array(value) {
        return Err(wrong_js_type(expected, value));
    }
    let tag = Reflect::get(value, &"tag".into())
        .ok()
        .and_then(|tag| tag.as_string())
        .ok_or_else(|| mismatch(expected, value, "missing string tag"))?;
    let payload = Reflect::get(value, &"val".into())
        .map_err(|error| mismatch(expected, &error, "could not read payload"))?;
    Ok((tag, payload))
}

fn lower_nested_result(
    value: Result<Option<Box<Val>>, Option<Box<Val>>>,
    ok: Option<&ValueType>,
    err: Option<&ValueType>,
    expected: &ValueType,
) -> Result<JsValue, CallError> {
    let (tag, value, ty) = match value {
        Ok(value) => ("ok", value, ok),
        Err(value) => ("err", value, err),
    };
    let payload = match (value, ty) {
        (Some(value), Some(ty)) => lower(*value, ty)?,
        (None, None) => JsValue::UNDEFINED,
        (value, _) => {
            return Err(mismatch(expected, &value, "wrong payload for result case"));
        }
    };
    tagged(tag, Some(payload), expected)
}

fn lift_nested_result(
    value: JsValue,
    ok: Option<&ValueType>,
    err: Option<&ValueType>,
    expected: &ValueType,
) -> Result<Val, CallError> {
    let (tag, payload) = tagged_parts(&value, expected)?;
    let lift_payload = |ty: Option<&ValueType>| {
        ty.map(|ty| lift(payload.clone(), ty).map(Box::new))
            .transpose()
    };
    match tag.as_str() {
        "ok" => lift_payload(ok).map(|value| Val::Result(Ok(value))),
        "err" => lift_payload(err).map(|value| Val::Result(Err(value))),
        _ => Err(mismatch(expected, &value, "unknown result case")),
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
    fn round_trips_records_with_jco_field_names() {
        let ty = ValueType::Record(vec![
            crate::types::FieldType {
                name: "signed-8".to_owned(),
                js_name: "signed8".to_owned(),
                ty: ValueType::S8,
            },
            crate::types::FieldType {
                name: "tags".to_owned(),
                js_name: "tags".to_owned(),
                ty: ValueType::List(Box::new(ValueType::String)),
            },
        ]);
        let values = vec![Val::Record(vec![
            ("signed-8".to_owned(), Val::S8(-8)),
            ("tags".to_owned(), Val::List(vec![Val::from("wasm")])),
        ])];
        let signature = FunctionType {
            params: vec![ty],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(
            Reflect::get(&lowered.get(0), &"signed8".into())
                .unwrap()
                .as_f64(),
            Some(-8.0)
        );
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn refuses_a_missing_record_field() {
        let error = lift_error(
            Object::new().into(),
            ValueType::Record(vec![crate::types::FieldType {
                name: "title".to_owned(),
                js_name: "title".to_owned(),
                ty: ValueType::String,
            }]),
        );
        assert!(error.contains("WIT `record`"), "{error}");
        assert!(error.contains("missing field `title`"), "{error}");
    }

    #[wasm_bindgen_test]
    fn round_trips_variants_and_enums() {
        let cases = vec![
            crate::types::CaseType {
                name: "none".to_owned(),
                ty: None,
            },
            crate::types::CaseType {
                name: "text".to_owned(),
                ty: Some(ValueType::String),
            },
        ];
        let values = vec![
            Val::Variant {
                case: "text".to_owned(),
                value: Some(Box::new(Val::from("diagram"))),
            },
            Val::Enum("upbeat".to_owned()),
        ];
        let signature = FunctionType {
            params: vec![
                ValueType::Variant(cases),
                ValueType::Enum(vec!["neutral".to_owned(), "upbeat".to_owned()]),
            ],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn refuses_an_unknown_variant_case() {
        let error = lower_args(
            vec![Val::Variant {
                case: "missing".to_owned(),
                value: None,
            }],
            &FunctionType {
                params: vec![ValueType::Variant(Vec::new())],
                result: None,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("WIT `variant`"), "{error}");
        assert!(error.contains("missing"), "{error}");
        assert!(error.contains("unknown variant case"), "{error}");
    }

    #[wasm_bindgen_test]
    fn round_trips_flags() {
        let values = vec![Val::Flags(vec![
            "short-form".to_owned(),
            "detailed".to_owned(),
        ])];
        let signature = FunctionType {
            params: vec![ValueType::Flags(vec![
                "short-form".to_owned(),
                "detailed".to_owned(),
            ])],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(
            Reflect::get(&lowered.get(0), &"shortForm".into())
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn refuses_an_unknown_flag() {
        let error = lower_args(
            vec![Val::Flags(vec!["missing".to_owned()])],
            &FunctionType {
                params: vec![ValueType::Flags(vec!["known".to_owned()])],
                result: None,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("WIT `flags`"), "{error}");
        assert!(error.contains("missing"), "{error}");
        assert!(error.contains("unknown flag"), "{error}");
    }

    #[wasm_bindgen_test]
    fn round_trips_options_including_nested_options() {
        let values = vec![
            Val::Option(None),
            Val::Option(Some(Box::new(Val::from("subtitle")))),
            Val::Option(Some(Box::new(Val::Option(None)))),
            Val::Option(Some(Box::new(Val::Option(Some(Box::new(Val::U32(7))))))),
        ];
        let string = ValueType::Option(Box::new(ValueType::String));
        let nested = ValueType::Option(Box::new(ValueType::Option(Box::new(ValueType::U32))));
        let signature = FunctionType {
            params: vec![string.clone(), string, nested.clone(), nested],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert!(lowered.get(0).is_undefined());
        assert_eq!(
            Reflect::get(&lowered.get(2), &"tag".into())
                .unwrap()
                .as_string(),
            Some("some".to_owned())
        );
        assert_eq!(lift_args(&lowered, &signature).unwrap(), values);
    }

    #[wasm_bindgen_test]
    fn round_trips_nested_results() {
        let values = vec![
            Val::Result(Ok(Some(Box::new(Val::Option(None))))),
            Val::Result(Err(Some(Box::new(Val::from("denied"))))),
            Val::Result(Ok(None)),
        ];
        let signature = FunctionType {
            params: vec![
                ValueType::Result {
                    ok: Some(Box::new(ValueType::Option(Box::new(ValueType::U64)))),
                    err: Some(Box::new(ValueType::String)),
                },
                ValueType::Result {
                    ok: Some(Box::new(ValueType::Option(Box::new(ValueType::U64)))),
                    err: Some(Box::new(ValueType::String)),
                },
                ValueType::Result {
                    ok: None,
                    err: None,
                },
            ],
            result: None,
        };
        let lowered = lower_args(values.clone(), &signature).unwrap();
        assert_eq!(
            Reflect::get(&lowered.get(1), &"tag".into())
                .unwrap()
                .as_string(),
            Some("err".to_owned())
        );
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
