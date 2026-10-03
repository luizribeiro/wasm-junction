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

pub(super) fn add_directory_stream_context(
    args: &mut Vals,
    store: &mut StoreData,
) -> Result<(), CallError> {
    let Some(Val::Resource(resource)) = args.first() else {
        return Err(CallError::refused("expected directory-entry-stream handle"));
    };
    validate_borrowed::<types::DirectoryEntryStream>(&args[0], store)?;
    let preopen = store
        .directory_stream_preopen(resource.id())
        .ok_or_else(|| {
            CallError::refused(format!(
                "unknown directory-entry-stream handle {}",
                resource.id()
            ))
        })?
        .to_owned();
    args.push(Val::String(preopen));
    Ok(())
}

pub(super) fn validate_directory_stream_context(
    args: &[Val],
    store: &mut StoreData,
) -> Result<(), CallError> {
    let Some(Val::String(context)) = args.last() else {
        return Err(CallError::refused(
            "missing directory-entry-stream preopen context",
        ));
    };
    let mut expected = args[..1].to_vec();
    add_directory_stream_context(&mut expected, store)?;
    match expected.last() {
        Some(Val::String(expected)) if expected == context => Ok(()),
        _ => Err(CallError::refused(
            "directory-entry-stream preopen context does not match",
        )),
    }
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
                        store.data_mut() $(, $arg)*);
                    let result = convert(store.data_mut(), result)?;
                    Ok(vec![result.to_val()])
                });
                let outcome = trampoline::gate(&mut store, INTERFACE, $name, args, real).await;
                Ok((finish_result::<$ok>(outcome)?,))
            }),
        )?;
    };
    (@call async, $method:path, $store:expr $(, $arg:ident)*) => {
        $method(&mut views::filesystem($store) $(, $arg)*).await
    };
    (@call sync, $method:path, $store:expr $(, $arg:ident)*) => {
        $method(&mut views::filesystem($store) $(, $arg)*)
    };
    (@call store_async, $method:path, $store:expr $(, $arg:ident)*) => {
        $method($store $(, $arg)*).await
    };
    (@call store_sync, $method:path, $store:expr $(, $arg:ident)*) => {
        $method($store $(, $arg)*)
    };
}

pub(super) use gate_fs;
