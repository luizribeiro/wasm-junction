use wasm_junction_core::{CallError, CallErrorKind, Val, Vals};
use wasmtime_wasi::p2::FsResult;
use wasmtime_wasi::p2::bindings::filesystem::types::{self, ErrorCode};

use super::super::{FromVal, finish, validate_borrowed, views};
use crate::engine::StoreData;

fn descriptor_preopen(value: &Val, store: &mut StoreData) -> Result<String, CallError> {
    validate_borrowed::<types::Descriptor>(value, store)?;
    let Val::Resource(resource) = value else {
        return Err(CallError::refused("expected descriptor handle"));
    };
    store
        .descriptor_preopen(resource.id())
        .map(str::to_owned)
        .ok_or_else(|| CallError::refused(format!("unknown descriptor handle {}", resource.id())))
}

pub(super) fn add_context(
    args: &mut Vals,
    descriptor_positions: &[usize],
    store: &mut StoreData,
) -> Result<(), CallError> {
    let contexts = descriptor_positions
        .iter()
        .map(|position| {
            args.get(*position)
                .ok_or_else(|| CallError::trap("missing descriptor argument"))
                .and_then(|value| descriptor_preopen(value, store))
                .map(Val::String)
        })
        .collect::<Result<Vec<_>, _>>()?;
    args.extend(contexts);
    Ok(())
}

pub(super) fn validate_context(
    args: &[Val],
    descriptor_positions: &[usize],
    store: &mut StoreData,
) -> Result<(), CallError> {
    let context_start = args
        .len()
        .checked_sub(descriptor_positions.len())
        .ok_or_else(|| CallError::refused("missing descriptor preopen context"))?;
    for (context_index, descriptor_position) in descriptor_positions.iter().enumerate() {
        let descriptor = args
            .get(*descriptor_position)
            .ok_or_else(|| CallError::refused("missing descriptor argument"))?;
        let expected = descriptor_preopen(descriptor, store)?;
        match args.get(context_start + context_index) {
            Some(Val::String(actual)) if actual == &expected => {}
            _ => {
                return Err(CallError::refused(
                    "descriptor preopen context does not match",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn convert<T>(
    store: &mut StoreData,
    result: FsResult<T>,
) -> Result<Result<T, ErrorCode>, CallError> {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(error) => types::Host::convert_error_code(&mut views::filesystem(store), error)
            .map(Err)
            .map_err(|error| CallError::trap(error.to_string())),
    }
}

pub(super) fn finish_result<T: FromVal>(
    outcome: Result<Vals, CallError>,
) -> wasmtime::Result<Result<T, ErrorCode>> {
    match outcome {
        Err(error) if error.kind() == CallErrorKind::Refused => Ok(Err(ErrorCode::Access)),
        outcome => finish(outcome),
    }
}

macro_rules! gate_fs {
    ($linker:ident, $name:literal, $method:path, $mode:ident, [$($position:expr),+],
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        $linker.instance(INTERFACE)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(vec![$($arg.to_val()),*], invocation);
                add_context(&mut args, &[$($position),+], store.data_mut())
                    .map_err(wasmtime::Error::new)?;
                let real: Real = |mut store, args| Box::pin(async move {
                    validate_context(&args, &[$($position),+], store.data_mut())?;
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| super::shape("another argument"))?
                    )?;)*
                    let result = gate_fs!(@call $mode, $method,
                        &mut views::filesystem(store.data_mut()) $(, $arg)*);
                    let result = convert(store.data_mut(), result)?;
                    Ok(vec![result.to_val()])
                });
                let outcome = trampoline::gate(&mut store, INTERFACE, $name, args, real).await;
                Ok((finish_result::<$ok>(outcome)?,))
            }),
        )?;
    };
    (@call async, $method:path, $view:expr $(, $arg:ident)*) => {
        $method($view $(, $arg)*).await
    };
    (@call sync, $method:path, $view:expr $(, $arg:ident)*) => {
        $method($view $(, $arg)*)
    };
}

pub(super) use gate_fs;
