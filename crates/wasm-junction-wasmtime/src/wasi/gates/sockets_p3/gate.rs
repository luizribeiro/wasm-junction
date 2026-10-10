use wasm_junction_core::{CallError, CallErrorKind, Vals};
use wasmtime::component::Linker;
use wasmtime_wasi::p3::bindings::sockets::{ip_name_lookup, types};
use wasmtime_wasi::sockets::{WasiSockets, WasiSocketsView};

use super::{
    FromVal, LOOKUP_INTERFACE, RealConcurrent, StoreData, ToVal, p3_result_value, require_sockets,
    scope_values, shape, trampoline,
};

pub(super) fn finish_result<T: FromVal>(
    outcome: Result<Vals, CallError>,
) -> wasmtime::Result<Result<T, types::ErrorCode>> {
    match outcome {
        Err(error) if error.kind() == CallErrorKind::Refused => {
            Ok(Err(types::ErrorCode::AccessDenied))
        }
        Err(error) => Err(wasmtime::Error::new(error)),
        Ok(values) => super::decode_p3_result(values).map_err(wasmtime::Error::new),
    }
}

macro_rules! gate_socket {
    ($linker:ident, $iface:expr, $name:literal, $method:path, $mode:ident, $validate:expr,
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        $linker.instance($iface)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = super::scope_values(vec![$($arg.to_val()),*], invocation);
                super::add_handle_contexts(&mut args, store.data());
                let real: super::Real = |mut store, args| Box::pin(async move {
                    super::require_sockets(store.data())?;
                    $validate(&args, store.data_mut())?;
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| super::shape("another argument"))?
                    )?;)*
                    let result = $crate::wasi::gates::sockets::gate::call_socket!(
                        $mode, $method, store.data_mut() $(, $arg)*
                    );
                    let result = super::convert_trappable(result)?;
                    let invocation = store.data().context.invocation_id()
                        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                    Ok(super::scope_values(vec![super::p3_result_value(result)], invocation))
                });
                let outcome = super::trampoline::gate(
                    &mut store, $iface, $name, args, real,
                ).await;
                Ok(($crate::wasi::gates::sockets_p3::gate::finish_result::<$ok>(outcome)?,))
            }),
        )?;
    };
}

macro_rules! gate_socket_concurrent {
    ($linker:ident, $name:literal, $method:path, $validate:expr,
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        $linker.instance(super::INTERFACE)?.func_wrap_concurrent(
            $name,
            |accessor, ($($arg,)*): ($($ty,)*)| Box::pin(async move {
                let invocation = accessor.with(|mut access| access.get().context.invocation_id())
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = super::scope_values(vec![$($arg.to_val()),*], invocation);
                accessor.with(|mut access| super::add_handle_contexts(&mut args, access.get()));
                let real: super::RealConcurrent = |accessor, args| Box::pin(async move {
                    accessor.with(|mut access| super::require_sockets(access.get()))?;
                    accessor.with(|mut access| $validate(&args, access.get()))?;
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| super::shape("another argument"))?
                    )?;)*
                    let view = accessor.with_getter::<WasiSockets>(WasiSocketsView::sockets);
                    let result = $method(&view $(, $arg)*).await;
                    let result = super::convert_trappable(result)?;
                    let invocation = accessor.with(|mut access| access.get().context.invocation_id())
                        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                    Ok(super::scope_values(vec![super::p3_result_value(result)], invocation))
                });
                let outcome = super::trampoline::gate_concurrent(
                    accessor, super::INTERFACE, $name, args, real,
                ).await;
                Ok(($crate::wasi::gates::sockets_p3::gate::finish_result::<$ok>(outcome)?,))
            }),
        )?;
    };
}

macro_rules! gate_socket_value {
    ($linker:ident, $name:literal, $method:path, $validate:expr,
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        $linker.instance(super::INTERFACE)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = super::scope_values(vec![$($arg.to_val()),*], invocation);
                super::add_handle_contexts(&mut args, store.data());
                let real: super::Real = |mut store, args| Box::pin(async move {
                    super::require_sockets(store.data())?;
                    $validate(&args, store.data_mut())?;
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_val(
                        args.next().ok_or_else(|| super::shape("another argument"))?
                    )?;)*
                    let value = $method(&mut super::views::sockets(store.data_mut()) $(, $arg)*)
                        .map_err(|error| CallError::trap(error.to_string()))?;
                    Ok(vec![value.to_val()])
                });
                let outcome = super::trampoline::gate(
                    &mut store, super::INTERFACE, $name, args, real,
                ).await;
                Ok((super::finish::<$ok>(outcome)?,))
            }),
        )?;
    };
}

pub(super) use gate_socket;
pub(super) use gate_socket_concurrent;
pub(super) use gate_socket_value;

fn decode_lookup(
    outcome: Result<Vals, CallError>,
) -> wasmtime::Result<Result<Vec<types::IpAddress>, ip_name_lookup::ErrorCode>> {
    match outcome {
        Err(error) if error.kind() == CallErrorKind::Refused => {
            Ok(Err(ip_name_lookup::ErrorCode::AccessDenied))
        }
        Err(error) => Err(wasmtime::Error::new(error)),
        Ok(values) => super::decode_p3_result(values).map_err(wasmtime::Error::new),
    }
}

pub(super) fn add_lookup(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(LOOKUP_INTERFACE)?.func_wrap_concurrent(
        "resolve-addresses",
        |accessor, (name,): (String,)| {
            Box::pin(async move {
                let invocation = accessor
                    .with(|mut access| access.get().context.invocation_id())
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = scope_values(vec![name.to_val()], invocation);
                let real: RealConcurrent = |accessor, mut args| {
                    Box::pin(async move {
                        accessor.with(|mut access| require_sockets(access.get()))?;
                        let name = String::from_val(args.pop().ok_or_else(|| shape("name"))?)?;
                        let view = accessor.with_getter::<WasiSockets>(WasiSocketsView::sockets);
                        let result = ip_name_lookup::HostWithStore::resolve_addresses(&view, name)
                            .await
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        Ok(vec![p3_result_value(result)])
                    })
                };
                let outcome = trampoline::gate_concurrent(
                    accessor,
                    LOOKUP_INTERFACE,
                    "resolve-addresses",
                    args,
                    real,
                )
                .await;
                Ok((decode_lookup(outcome)?,))
            })
        },
    )?;
    Ok(())
}
