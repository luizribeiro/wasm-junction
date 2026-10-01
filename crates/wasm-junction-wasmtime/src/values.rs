use wasm_junction::Val;
use wasmtime::component::Val as WasmtimeVal;

pub(crate) fn from_wasmtime(value: WasmtimeVal) -> Result<Val, wasmtime::Error> {
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
        other => Err(wasmtime::Error::msg(format!(
            "unsupported component value: {other:?}"
        ))),
    }
}

pub(crate) fn to_wasmtime(value: Val) -> Result<WasmtimeVal, wasmtime::Error> {
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
        other => Err(wasmtime::Error::msg(format!(
            "unsupported framework value: {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: Val) -> Val {
        from_wasmtime(to_wasmtime(value).unwrap()).unwrap()
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
}
