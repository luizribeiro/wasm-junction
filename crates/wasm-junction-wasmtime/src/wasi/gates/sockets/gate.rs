use wasm_junction_core::{CallError, CallErrorKind, Vals};
use wasmtime_wasi::p2::SocketResult;
use wasmtime_wasi::p2::bindings::sockets::network::{self, ErrorCode};

use super::super::{FromVal, finish, views};
use crate::engine::StoreData;

pub(super) fn convert<T>(
    store: &mut StoreData,
    result: SocketResult<T>,
) -> Result<Result<T, ErrorCode>, CallError> {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(error) => network::Host::convert_error_code(&mut views::sockets(store), error)
            .map(Err)
            .map_err(|error| CallError::trap(error.to_string())),
    }
}

pub(super) fn finish_result<T: FromVal>(
    outcome: Result<Vals, CallError>,
) -> wasmtime::Result<Result<T, ErrorCode>> {
    match outcome {
        Err(error) if error.kind() == CallErrorKind::Refused => Ok(Err(ErrorCode::AccessDenied)),
        outcome => finish(outcome),
    }
}

macro_rules! call_socket {
    (view_async, $method:path, $store:expr $(, $arg:ident)*) => {
        $method(&mut $crate::wasi::gates::views::sockets($store) $(, $arg)*).await
    };
    (view_sync, $method:path, $store:expr $(, $arg:ident)*) => {
        $method(&mut $crate::wasi::gates::views::sockets($store) $(, $arg)*)
    };
    (store_async, $method:path, $store:expr $(, $arg:ident)*) => {
        $method($store $(, $arg)*).await
    };
    (store_sync, $method:path, $store:expr $(, $arg:ident)*) => {
        $method($store $(, $arg)*)
    };
}

macro_rules! socket_options {
    ($gate:ident, $linker:ident, $interface:expr, $socket:ty, $validate:path, $(
        $get_name:literal, $get:path, $get_ty:ty,
        $set_name:literal, $set:path, $set_ty:ty;
    )+) => {$(
        $gate!($linker, $interface, $get_name, $get, view_sync, $validate,
            (socket: wasmtime::component::Resource<$socket>) -> $get_ty);
        $gate!($linker, $interface, $set_name, $set, view_sync, $validate,
            (socket: wasmtime::component::Resource<$socket>, value: $set_ty) -> ());
    )+};
}

macro_rules! gate_socket {
    ($linker:ident, $iface:expr, $name:literal, $method:path, $mode:ident, $validate:expr,
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        $linker.instance($iface)?.func_wrap_async(
            $name,
            |mut store, ($($arg,)*): ($($ty,)*)| Box::new(async move {
                let invocation = store.data().context.invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = $crate::wasi::gates::scope_values(
                    vec![$($crate::wasi::gates::ToVal::to_val($arg)),*], invocation,
                );
                super::add_handle_contexts(&mut args, store.data());
                let real: $crate::wasi::trampoline::Real = |mut store, args| Box::pin(async move {
                    $validate(&args, store.data_mut())?;
                    #[allow(unused_mut, unused_variables)]
                    let mut args = args.into_iter();
                    $(let $arg = <$ty as $crate::wasi::gates::FromVal>::from_val(
                        args.next().ok_or_else(|| super::shape("another argument"))?
                    )?;)*
                    let result = $crate::wasi::gates::sockets::gate::call_socket!($mode, $method,
                        store.data_mut() $(, $arg)*);
                    let result = $crate::wasi::gates::sockets::gate::convert(
                        store.data_mut(), result,
                    )?;
                    let invocation = store.data().context.invocation_id()
                        .ok_or_else(|| wasm_junction_core::CallError::trap(
                            "WASI call has no invocation id"
                        ))?;
                    Ok($crate::wasi::gates::scope_values(
                        vec![$crate::wasi::gates::ToVal::to_val(result)], invocation,
                    ))
                });
                let outcome = $crate::wasi::trampoline::gate(
                    &mut store, $iface, $name, args, real,
                ).await;
                Ok(($crate::wasi::gates::sockets::gate::finish_result::<$ok>(outcome)?,))
            }),
        )?;
    };
}

pub(in crate::wasi::gates) use call_socket;
pub(super) use gate_socket;
pub(in crate::wasi::gates) use socket_options;
