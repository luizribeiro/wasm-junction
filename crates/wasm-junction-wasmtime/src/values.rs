use wasm_junction_core::{FutureHandle, Resource, ResourceOwnership, StreamHandle, Val};
use wasmtime::component::{
    FutureAny, ResourceAny, ResourceType, StreamAny, Type, Val as WasmtimeVal,
};

#[derive(Clone, Copy)]
pub(crate) struct ExpectedResource {
    pub(crate) ownership: ResourceOwnership,
    pub(crate) ty: ResourceType,
}

pub(crate) enum LiftValue {
    Future(FutureAny),
    Resource(ResourceAny),
    Stream(StreamAny, Option<Type>),
}

pub(crate) enum LowerValue {
    Future(FutureHandle),
    Resource(Resource, Option<ExpectedResource>),
    Stream(StreamHandle, Type),
}

pub(crate) fn from_wasmtime(
    value: WasmtimeVal,
    expected: Option<&Type>,
    store: &mut impl FnMut(LiftValue) -> Result<Val, wasmtime::Error>,
) -> Result<Val, wasmtime::Error> {
    match value {
        WasmtimeVal::Bool(value) => Ok(Val::Bool(value)),
        WasmtimeVal::S8(value) => Ok(Val::S8(value)),
        WasmtimeVal::U8(value) => Ok(Val::U8(value)),
        WasmtimeVal::S16(value) => Ok(Val::S16(value)),
        WasmtimeVal::U16(value) => Ok(Val::U16(value)),
        WasmtimeVal::S32(value) => Ok(Val::S32(value)),
        WasmtimeVal::U32(value) => Ok(Val::U32(value)),
        WasmtimeVal::S64(value) => Ok(Val::S64(value)),
        WasmtimeVal::U64(value) => Ok(Val::U64(value)),
        WasmtimeVal::Float32(value) => Ok(Val::F32(value)),
        WasmtimeVal::Float64(value) => Ok(Val::F64(value)),
        WasmtimeVal::Char(value) => Ok(Val::Char(value)),
        WasmtimeVal::String(value) => Ok(Val::String(value)),
        WasmtimeVal::List(values) if is_byte_list(expected) => values
            .into_iter()
            .map(|value| match value {
                WasmtimeVal::U8(value) => Ok(value),
                other => Err(wasmtime::Error::msg(format!(
                    "expected u8 in list<u8>, got {other:?}"
                ))),
            })
            .collect::<Result<_, _>>()
            .map(Val::Bytes),
        WasmtimeVal::List(values) => {
            let ty = list_element_type(expected);
            values
                .into_iter()
                .map(|value| from_wasmtime(value, ty.as_ref(), store))
                .collect::<Result<_, _>>()
                .map(Val::List)
        }
        WasmtimeVal::Tuple(values) => values
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                let ty = tuple_element_type(expected, index);
                from_wasmtime(value, ty.as_ref(), store)
            })
            .collect::<Result<_, _>>()
            .map(Val::Tuple),
        WasmtimeVal::Record(fields) => fields
            .into_iter()
            .map(|(name, value)| {
                let ty = record_field_type(expected, &name);
                Ok((name, from_wasmtime(value, ty.as_ref(), store)?))
            })
            .collect::<Result<_, _>>()
            .map(Val::Record),
        WasmtimeVal::Variant(case, value) => {
            let ty = variant_case_type(expected, &case);
            Ok(Val::Variant {
                case,
                value: value
                    .map(|value| from_wasmtime(*value, ty.as_ref(), store).map(Box::new))
                    .transpose()?,
            })
        }
        WasmtimeVal::Enum(case) => Ok(Val::Enum(case)),
        WasmtimeVal::Flags(names) => Ok(Val::Flags(names)),
        WasmtimeVal::Option(value) => Ok(Val::Option(
            value
                .map(|value| {
                    let ty = option_type(expected);
                    from_wasmtime(*value, ty.as_ref(), store).map(Box::new)
                })
                .transpose()?,
        )),
        WasmtimeVal::Result(result) => Ok(Val::Result(match result {
            Ok(value) => Ok(value
                .map(|value| {
                    let ty = result_type(expected, true);
                    from_wasmtime(*value, ty.as_ref(), store).map(Box::new)
                })
                .transpose()?),
            Err(value) => Err(value
                .map(|value| {
                    let ty = result_type(expected, false);
                    from_wasmtime(*value, ty.as_ref(), store).map(Box::new)
                })
                .transpose()?),
        })),
        WasmtimeVal::Resource(value) => store(LiftValue::Resource(value)),
        WasmtimeVal::Future(value) => store(LiftValue::Future(value)),
        WasmtimeVal::Stream(value) => store(LiftValue::Stream(value, stream_item_type(expected))),
        other => Err(wasmtime::Error::msg(format!(
            "unsupported component value: {other:?}"
        ))),
    }
}

pub(crate) fn to_wasmtime(
    value: Val,
    expected: Option<&Type>,
    store: &mut impl FnMut(LowerValue) -> Result<WasmtimeVal, wasmtime::Error>,
) -> Result<WasmtimeVal, wasmtime::Error> {
    match value {
        Val::Bool(value) => Ok(WasmtimeVal::Bool(value)),
        Val::S8(value) => Ok(WasmtimeVal::S8(value)),
        Val::U8(value) => Ok(WasmtimeVal::U8(value)),
        Val::S16(value) => Ok(WasmtimeVal::S16(value)),
        Val::U16(value) => Ok(WasmtimeVal::U16(value)),
        Val::S32(value) => Ok(WasmtimeVal::S32(value)),
        Val::U32(value) => Ok(WasmtimeVal::U32(value)),
        Val::S64(value) => Ok(WasmtimeVal::S64(value)),
        Val::U64(value) => Ok(WasmtimeVal::U64(value)),
        Val::F32(value) => Ok(WasmtimeVal::Float32(value)),
        Val::F64(value) => Ok(WasmtimeVal::Float64(value)),
        Val::Char(value) => Ok(WasmtimeVal::Char(value)),
        Val::String(value) => Ok(WasmtimeVal::String(value)),
        Val::Bytes(values) => lower_bytes(values, expected),
        Val::List(values) => {
            let ty = list_element_type(expected);
            convert_values(values, ty.as_ref(), store).map(WasmtimeVal::List)
        }
        Val::Tuple(values) => {
            let types = match expected {
                Some(Type::Tuple(ty)) => ty.types().collect(),
                _ => Vec::new(),
            };
            values
                .into_iter()
                .enumerate()
                .map(|(index, value)| to_wasmtime(value, types.get(index), store))
                .collect::<Result<_, _>>()
                .map(WasmtimeVal::Tuple)
        }
        Val::Record(fields) => fields
            .into_iter()
            .map(|(name, value)| {
                let ty = match expected {
                    Some(Type::Record(ty)) => ty
                        .fields()
                        .find(|field| field.name == name)
                        .map(|field| field.ty),
                    _ => None,
                };
                Ok((name, to_wasmtime(value, ty.as_ref(), store)?))
            })
            .collect::<Result<_, _>>()
            .map(WasmtimeVal::Record),
        Val::Variant { case, value } => {
            let ty = match expected {
                Some(Type::Variant(ty)) => ty
                    .cases()
                    .find(|candidate| candidate.name == case)
                    .and_then(|case| case.ty),
                _ => None,
            };
            Ok(WasmtimeVal::Variant(
                case,
                value
                    .map(|value| to_wasmtime(*value, ty.as_ref(), store).map(Box::new))
                    .transpose()?,
            ))
        }
        Val::Enum(case) => Ok(WasmtimeVal::Enum(case)),
        Val::Flags(names) => Ok(WasmtimeVal::Flags(names)),
        Val::Option(value) => {
            let ty = match expected {
                Some(Type::Option(ty)) => Some(ty.ty()),
                _ => None,
            };
            Ok(WasmtimeVal::Option(
                value
                    .map(|value| to_wasmtime(*value, ty.as_ref(), store).map(Box::new))
                    .transpose()?,
            ))
        }
        Val::Result(result) => Ok(WasmtimeVal::Result(match result {
            Ok(value) => Ok(value
                .map(|value| {
                    let ty = match expected {
                        Some(Type::Result(ty)) => ty.ok(),
                        _ => None,
                    };
                    to_wasmtime(*value, ty.as_ref(), store).map(Box::new)
                })
                .transpose()?),
            Err(value) => Err(value
                .map(|value| {
                    let ty = match expected {
                        Some(Type::Result(ty)) => ty.err(),
                        _ => None,
                    };
                    to_wasmtime(*value, ty.as_ref(), store).map(Box::new)
                })
                .transpose()?),
        })),
        Val::Resource(value) => store(LowerValue::Resource(value, expected_resource(expected))),
        Val::Future(value) => store(LowerValue::Future(value)),
        Val::Stream(value) => lower_stream_value(value, expected, store),
        other => Err(wasmtime::Error::msg(format!(
            "unsupported framework value: {other:?}"
        ))),
    }
}

fn lower_bytes(values: Vec<u8>, expected: Option<&Type>) -> Result<WasmtimeVal, wasmtime::Error> {
    is_byte_list(expected)
        .then(|| WasmtimeVal::List(values.into_iter().map(WasmtimeVal::U8).collect()))
        .ok_or_else(|| wasmtime::Error::msg("expected list<u8> value type"))
}

fn is_byte_list(expected: Option<&Type>) -> bool {
    matches!(list_element_type(expected), Some(Type::U8))
}

fn tuple_element_type(expected: Option<&Type>, index: usize) -> Option<Type> {
    match expected {
        Some(Type::Tuple(ty)) => ty.types().nth(index),
        _ => None,
    }
}

fn record_field_type(expected: Option<&Type>, name: &str) -> Option<Type> {
    match expected {
        Some(Type::Record(ty)) => ty
            .fields()
            .find(|field| field.name == name)
            .map(|field| field.ty),
        _ => None,
    }
}

fn variant_case_type(expected: Option<&Type>, name: &str) -> Option<Type> {
    match expected {
        Some(Type::Variant(ty)) => ty
            .cases()
            .find(|case| case.name == name)
            .and_then(|case| case.ty),
        _ => None,
    }
}

fn option_type(expected: Option<&Type>) -> Option<Type> {
    match expected {
        Some(Type::Option(ty)) => Some(ty.ty()),
        _ => None,
    }
}

fn result_type(expected: Option<&Type>, ok: bool) -> Option<Type> {
    match expected {
        Some(Type::Result(ty)) if ok => ty.ok(),
        Some(Type::Result(ty)) => ty.err(),
        _ => None,
    }
}

fn list_element_type(expected: Option<&Type>) -> Option<Type> {
    match expected {
        Some(Type::List(ty)) => Some(ty.ty()),
        Some(Type::FixedLengthList(ty)) => Some(ty.ty()),
        _ => None,
    }
}

pub(crate) fn expected_resource(expected: Option<&Type>) -> Option<ExpectedResource> {
    match expected {
        Some(Type::Own(ty)) => Some(ExpectedResource {
            ownership: ResourceOwnership::Own,
            ty: *ty,
        }),
        Some(Type::Borrow(ty)) => Some(ExpectedResource {
            ownership: ResourceOwnership::Borrow,
            ty: *ty,
        }),
        _ => None,
    }
}

fn stream_item_type(expected: Option<&Type>) -> Option<Type> {
    match expected {
        Some(Type::Stream(stream)) => stream.ty(),
        _ => None,
    }
}

fn lower_stream_value(
    value: StreamHandle,
    expected: Option<&Type>,
    store: &mut impl FnMut(LowerValue) -> Result<WasmtimeVal, wasmtime::Error>,
) -> Result<WasmtimeVal, wasmtime::Error> {
    let item_type = stream_item_type(expected)
        .ok_or_else(|| wasmtime::Error::msg("expected stream value type"))?;
    if value.is_byte_stream() != (item_type == Type::U8) {
        return Err(wasmtime::Error::new(
            wasm_junction_core::CallError::refused("stream item type does not match its WIT type"),
        ));
    }
    store(LowerValue::Stream(value, item_type))
}

fn convert_values(
    values: Vec<Val>,
    expected: Option<&Type>,
    store: &mut impl FnMut(LowerValue) -> Result<WasmtimeVal, wasmtime::Error>,
) -> Result<Vec<WasmtimeVal>, wasmtime::Error> {
    values
        .into_iter()
        .map(|value| to_wasmtime(value, expected, store))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_junction_conformance::sample_note;
    use wasm_junction_core::Resource;

    fn round_trip(value: Val) -> Val {
        let value = to_wasmtime(value, None, &mut |_| unreachable!()).unwrap();
        from_wasmtime(value, None, &mut |_| unreachable!()).unwrap()
    }

    #[test]
    fn primitives_and_boundary_integers_round_trip() {
        let values = [
            Val::Bool(true),
            Val::S8(i8::MIN),
            Val::S8(i8::MAX),
            Val::U8(u8::MIN),
            Val::U8(u8::MAX),
            Val::S16(i16::MIN),
            Val::S16(i16::MAX),
            Val::U16(u16::MIN),
            Val::U16(u16::MAX),
            Val::S32(i32::MIN),
            Val::S32(i32::MAX),
            Val::U32(u32::MIN),
            Val::U32(u32::MAX),
            Val::S64(i64::MIN),
            Val::S64(i64::MAX),
            Val::U64(u64::MIN),
            Val::U64(u64::MAX),
            Val::Char('􏿿'),
            Val::String("notes".to_owned()),
        ];
        for value in values {
            assert_eq!(round_trip(value.clone()), value);
        }
    }

    #[test]
    fn floating_point_nan_round_trips() {
        let Val::F32(value) = round_trip(Val::F32(f32::NAN)) else {
            panic!("f32 changed shape");
        };
        assert!(value.is_nan());
        let Val::F64(value) = round_trip(Val::F64(f64::NAN)) else {
            panic!("f64 changed shape");
        };
        assert!(value.is_nan());
    }

    #[test]
    fn list_tuple_and_record_round_trip() {
        let value = Val::Record(vec![(
            "items".to_owned(),
            Val::List(vec![Val::Tuple(vec![Val::U32(7), Val::from("weekly")])]),
        )]);
        assert_eq!(round_trip(value.clone()), value);
        assert_eq!(round_trip(Val::List(Vec::new())), Val::List(Vec::new()));
    }

    #[test]
    fn variant_enum_and_flags_edge_cases_round_trip() {
        let values = [
            Val::Variant {
                case: "none".to_owned(),
                value: None,
            },
            Val::Variant {
                case: "count".to_owned(),
                value: Some(Box::new(Val::U32(3))),
            },
            Val::Enum("neutral".to_owned()),
            Val::Enum("upbeat".to_owned()),
            Val::Flags(Vec::new()),
            Val::Flags(vec!["concise".to_owned(), "detailed".to_owned()]),
        ];
        for value in values {
            assert_eq!(round_trip(value.clone()), value);
        }
    }

    #[test]
    fn option_and_payload_free_results_round_trip() {
        let values = [
            Val::Option(None),
            Val::Result(Err(None)),
            Val::Result(Ok(None)),
        ];
        for value in values {
            assert_eq!(round_trip(value.clone()), value);
        }
    }

    #[test]
    fn every_plain_shape_round_trips_both_converters() {
        let note = sample_note();
        assert_eq!(round_trip(note.clone()), note);
    }

    #[test]
    fn nested_resources_use_the_store_aware_conversion() {
        let value = Val::Option(Some(Box::new(Val::Resource(Resource::owned(
            "example:resources/host@1.0.0",
            "session",
            4,
        )))));
        let error = to_wasmtime(value, None, &mut |value| {
            let LowerValue::Resource(resource, _) = value else {
                unreachable!();
            };
            Err(wasmtime::Error::msg(format!(
                "saw {}/{}:{}",
                resource.interface(),
                resource.name(),
                resource.id()
            )))
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "saw example:resources/host@1.0.0/session:4"
        );
    }

    #[test]
    fn nested_streams_require_the_declared_value_type() {
        let value = Val::Option(Some(Box::new(Val::from(
            wasm_junction_core::OutputStream::from_bytes(b"nested"),
        ))));
        let error = to_wasmtime(value, None, &mut |_| unreachable!()).unwrap_err();
        assert_eq!(error.to_string(), "expected stream value type");
    }
}
