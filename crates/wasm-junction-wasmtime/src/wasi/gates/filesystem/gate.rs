use wasm_junction_core::{CallError, CallErrorKind, Val, Vals};
use wasmtime_wasi::p2::FsResult;
use wasmtime_wasi::p2::bindings::filesystem::types::{self, ErrorCode};

use super::super::{FromVal, finish, validate_borrowed, views};
use crate::engine::StoreData;

pub(in crate::wasi::gates) fn join_guest_path(parent: &str, path: &str) -> Option<String> {
    let absolute = parent.starts_with('/');
    let mut components = Vec::new();
    for component in parent.split('/').chain(path.split('/')) {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            component => components.push(component),
        }
    }
    let joined = components.join("/");
    if absolute {
        Some(format!("/{joined}"))
    } else if joined.is_empty() {
        Some(".".to_owned())
    } else {
        Some(joined)
    }
}

fn descriptor_guest_path(
    value: &Val,
    interface: &str,
    store: &mut StoreData,
) -> Result<String, CallError> {
    super::super::validate_borrowed_resource::<types::Descriptor>(
        value,
        interface,
        super::DESCRIPTOR,
        store,
    )?;
    let Val::Resource(resource) = value else {
        return Err(CallError::refused("expected descriptor handle"));
    };
    store
        .descriptor_guest_path(resource.id())
        .map(str::to_owned)
        .ok_or_else(|| CallError::refused(format!("unknown descriptor handle {}", resource.id())))
}

pub(super) fn add_context(
    args: &mut Vals,
    descriptor_positions: &[usize],
    store: &mut StoreData,
) -> Result<(), CallError> {
    add_context_for(super::INTERFACE, args, descriptor_positions, store)
}

pub(in crate::wasi::gates) fn add_context_for(
    interface: &str,
    args: &mut Vals,
    descriptor_positions: &[usize],
    store: &mut StoreData,
) -> Result<(), CallError> {
    let contexts = descriptor_positions
        .iter()
        .map(|position| {
            args.get(*position)
                .ok_or_else(|| CallError::trap("missing descriptor argument"))
                .and_then(|value| descriptor_guest_path(value, interface, store))
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
    validate_context_for(super::INTERFACE, args, descriptor_positions, store)
}

pub(in crate::wasi::gates) fn validate_context_for(
    interface: &str,
    args: &[Val],
    descriptor_positions: &[usize],
    store: &mut StoreData,
) -> Result<(), CallError> {
    let context_start = args
        .len()
        .checked_sub(descriptor_positions.len())
        .ok_or_else(|| CallError::refused("missing descriptor guest path context"))?;
    for (context_index, descriptor_position) in descriptor_positions.iter().enumerate() {
        let descriptor = args
            .get(*descriptor_position)
            .ok_or_else(|| CallError::refused("missing descriptor argument"))?;
        let expected = descriptor_guest_path(descriptor, interface, store)?;
        match args.get(context_start + context_index) {
            Some(Val::String(actual)) if actual == &expected => {}
            _ => {
                return Err(CallError::refused(
                    "descriptor guest path context does not match",
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
    let guest_path = store
        .directory_stream_guest_path(resource.id())
        .ok_or_else(|| {
            CallError::refused(format!(
                "unknown directory-entry-stream handle {}",
                resource.id()
            ))
        })?
        .to_owned();
    args.push(Val::String(guest_path));
    Ok(())
}

pub(super) fn validate_directory_stream_context(
    args: &[Val],
    store: &mut StoreData,
) -> Result<(), CallError> {
    let Some(Val::String(context)) = args.last() else {
        return Err(CallError::refused(
            "missing directory-entry-stream guest path context",
        ));
    };
    let mut expected = args[..1].to_vec();
    add_directory_stream_context(&mut expected, store)?;
    match expected.last() {
        Some(Val::String(expected)) if expected == context => Ok(()),
        _ => Err(CallError::refused(
            "directory-entry-stream guest path context does not match",
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wasm_junction_core::{
        BoxFuture, CallError, CallErrorKind, ImportDispatcher, ImportTarget, InvocationContext,
        InvocationId, Resource as JunctionResource, Val, Vals,
    };
    use wasmtime_wasi::p2::bindings::filesystem::preopens;
    use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

    use super::{add_context, join_guest_path};
    use crate::engine::StoreData;
    use crate::wasi::WasiState;
    use crate::wasi::gates::filesystem::{DESCRIPTOR, INTERFACE};
    use crate::wasi::gates::views;

    struct NoCalls;

    impl ImportDispatcher for NoCalls {
        fn call(
            &self,
            _context: InvocationContext,
            _caller: Arc<str>,
            _interface: Arc<str>,
            _function: Arc<str>,
            _args: Vals,
        ) -> BoxFuture<'_, Result<Vals, CallError>> {
            Box::pin(async { Err(CallError::trap("unexpected imported call")) })
        }

        fn call_engine(
            &self,
            _context: InvocationContext,
            _caller: Arc<str>,
            _interface: Arc<str>,
            _function: Arc<str>,
            _args: Vals,
            _target: Arc<dyn ImportTarget>,
        ) -> BoxFuture<'_, Result<Vals, CallError>> {
            Box::pin(async { Err(CallError::trap("unexpected engine call")) })
        }

        fn drop_resource(
            &self,
            _context: InvocationContext,
            _caller: Arc<str>,
            _resource: JunctionResource,
        ) -> BoxFuture<'_, Result<(), CallError>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn joins_and_normalizes_guest_paths_lexically() {
        assert_eq!(
            join_guest_path("/notes/private", "./nested/../passwords.md"),
            Some("/notes/private/passwords.md".to_owned())
        );
        assert_eq!(join_guest_path("/notes", ""), Some("/notes".to_owned()));
        assert_eq!(join_guest_path("/notes", "."), Some("/notes".to_owned()));
        assert_eq!(
            join_guest_path("/notes/", "private/"),
            Some("/notes/private".to_owned())
        );
        assert_eq!(
            join_guest_path("/notes/private", ".."),
            Some("/notes".to_owned())
        );
        assert_eq!(join_guest_path("/", "notes"), Some("/notes".to_owned()));
        assert_eq!(join_guest_path(".", "notes"), Some("notes".to_owned()));
        assert_eq!(join_guest_path("/notes", "../../outside"), None);
    }

    #[test]
    fn p2_gate_refuses_a_descriptor_without_a_guest_path() {
        let mut builder = WasiCtxBuilder::new();
        builder
            .preopened_dir(std::env::temp_dir(), "/data", FsPerms::ReadWrite)
            .unwrap();
        let invocation = InvocationId::__from_counter(1);
        let context = InvocationContext::default().with_invocation_id(invocation);
        let mut store =
            StoreData::for_wasi_test(Arc::new(NoCalls), context, WasiState::new(builder.build()));
        let descriptor = preopens::Host::get_directories(&mut views::filesystem(&mut store))
            .unwrap()[0]
            .0
            .rep();
        store.set_descriptor_guest_path(descriptor, "/data".to_owned());
        store.remove_descriptor_guest_path(descriptor);
        let resource = JunctionResource::__borrowed_for_invocation(
            INTERFACE, DESCRIPTOR, descriptor, invocation,
        );
        let error = add_context(&mut vec![Val::Resource(resource)], &[0], &mut store).unwrap_err();
        assert_eq!(error.kind(), CallErrorKind::Refused);
    }
}
