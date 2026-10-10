use wasm_junction_core::{CallError, CallErrorKind, ChannelDirection, Val, Vals};
use wasmtime::AsContextMut;
use wasmtime::component::{Access, Linker, Resource, StreamReader};
use wasmtime_wasi::p3::bindings::sockets::types::{
    ErrorCode, HostTcpSocket, HostTcpSocketWithStore, IpAddressFamily, IpSocketAddress, TcpSocket,
};
use wasmtime_wasi::p3::sockets::SocketResult;
use wasmtime_wasi::sockets::{WasiSockets, WasiSocketsView};

use super::gate::{gate_socket, gate_socket_concurrent, gate_socket_value};
#[allow(
    clippy::wildcard_imports,
    reason = "TCP gates share the socket module's private gate machinery"
)]
use super::*;
use crate::streams::{lift_static_stream, lower_static_stream};

#[cfg(test)]
mod tests;

async fn bind(
    store: &mut StoreData,
    socket: Resource<TcpSocket>,
    address: IpSocketAddress,
) -> SocketResult<()> {
    require_sockets(store).map_err(wasmtime_wasi::p3::sockets::SocketError::trap)?;
    let id = socket.rep();
    HostTcpSocket::bind(&mut views::sockets(store), socket, address).await?;
    let actual =
        HostTcpSocket::get_local_address(&mut views::sockets(store), Resource::new_borrow(id))?;
    store.set_socket_local_address(id, actual.to_val());
    Ok(())
}

async fn connect(
    accessor: &wasmtime::component::Accessor<StoreData, WasiSockets>,
    socket: Resource<TcpSocket>,
    address: IpSocketAddress,
) -> SocketResult<()> {
    accessor
        .with(|mut access| require_sockets(access.as_context_mut().data()))
        .map_err(wasmtime_wasi::p3::sockets::SocketError::trap)?;
    let id = socket.rep();
    HostTcpSocketWithStore::connect(accessor, socket, address).await?;
    accessor.with(|mut access| {
        access
            .as_context_mut()
            .data_mut()
            .set_wasi_handle_context(id, address.to_val());
    });
    Ok(())
}

fn decode_socket(value: Val) -> Result<Resource<TcpSocket>, CallError> {
    resource_from_val(value, INTERFACE, TCP_SOCKET)
}

fn drop_socket(store: &mut StoreData, socket: Resource<TcpSocket>) -> wasmtime::Result<()> {
    HostTcpSocket::drop(&mut views::sockets(store), socket)
}

fn listen_real(
    mut store: wasmtime::StoreContextMut<'_, StoreData>,
    args: Vals,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    Box::pin(async move {
        require_sockets(store.data())?;
        validate_tcp(&args, store.data_mut())?;
        let socket = decode_socket(args.into_iter().next().ok_or_else(|| shape(TCP_SOCKET))?)?;
        let access =
            Access::<StoreData, WasiSockets>::new(store.as_context_mut(), WasiSocketsView::sockets);
        let result = HostTcpSocketWithStore::listen(access, socket).await;
        let result = convert_trappable(result)?;
        Ok(vec![match result {
            Ok(stream) => Val::Result(Ok(Some(Box::new(Val::Stream(
                lift_static_stream(
                    stream,
                    store.as_context_mut(),
                    ChannelDirection::HostToGuest,
                )
                .map_err(|error| CallError::trap(error.to_string()))?,
            ))))),
            Err(error) => p3_result_value::<(), _>(Err(error)),
        }])
    })
}

fn add_listen(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(INTERFACE)?.func_wrap_async(
        "[method]tcp-socket.listen",
        |mut store, (socket,): (Resource<TcpSocket>,)| {
            Box::new(async move {
                let invocation = store
                    .data()
                    .context
                    .invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(
                    vec![resource_to_val(&socket, INTERFACE, TCP_SOCKET)],
                    invocation,
                );
                add_handle_contexts(&mut args, store.data());
                let outcome = trampoline::gate(
                    &mut store,
                    INTERFACE,
                    "[method]tcp-socket.listen",
                    args,
                    listen_real,
                )
                .await;
                let outcome = match outcome {
                    Err(error) if error.kind() == CallErrorKind::Refused => {
                        return Ok((Result::<StreamReader<Resource<TcpSocket>>, _>::Err(
                            ErrorCode::AccessDenied,
                        ),));
                    }
                    Err(error) => return Err(wasmtime::Error::new(error)),
                    Ok(values) => values,
                };
                let [Val::Result(result)] =
                    <[Val; 1]>::try_from(outcome).map_err(|_| shape("listen result"))?
                else {
                    return Err(wasmtime::Error::new(shape("listen result")));
                };
                let result = match result {
                    Ok(Some(value)) => {
                        let Val::Stream(stream) = *value else {
                            return Err(wasmtime::Error::new(shape("socket stream")));
                        };
                        Ok(lower_static_stream(stream, store.as_context_mut())?)
                    }
                    Err(Some(value)) => Err(ErrorCode::from_val(*value)?),
                    _ => return Err(wasmtime::Error::new(shape("listen result"))),
                };
                Ok((result,))
            })
        },
    )?;
    Ok(())
}

macro_rules! tcp_options {
    ($linker:ident, $($get:literal, $get_method:path, $get_ty:ty,
        $set:literal, $set_method:path, $set_ty:ty;)+) => {$(
        gate_socket!($linker, $get, $get_method, view_sync, super::validate_tcp,
            (socket: Resource<TcpSocket>) -> $get_ty);
        gate_socket!($linker, $set, $set_method, view_sync, super::validate_tcp,
            (socket: Resource<TcpSocket>, value: $set_ty) -> ());
    )+};
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_drop!(
        linker,
        INTERFACE,
        TCP_SOCKET,
        "[drop]tcp-socket",
        TcpSocket,
        store,
        None,
        drop_socket
    );
    gate_socket!(linker, "[static]tcp-socket.create", HostTcpSocket::create, view_sync,
        super::validate_none, (family: IpAddressFamily) -> Resource<TcpSocket>);
    gate_socket!(linker, "[method]tcp-socket.bind", bind, store_async, super::validate_tcp,
        (socket: Resource<TcpSocket>, address: IpSocketAddress) -> ());
    gate_socket_concurrent!(linker, "[method]tcp-socket.connect", connect,
        super::validate_tcp, (socket: Resource<TcpSocket>, address: IpSocketAddress) -> ());
    add_listen(linker)?;
    gate_socket!(linker, "[method]tcp-socket.get-local-address",
        HostTcpSocket::get_local_address, view_sync, super::validate_tcp,
        (socket: Resource<TcpSocket>) -> IpSocketAddress);
    gate_socket!(linker, "[method]tcp-socket.get-remote-address",
        HostTcpSocket::get_remote_address, view_sync, super::validate_tcp,
        (socket: Resource<TcpSocket>) -> IpSocketAddress);
    gate_socket_value!(linker, "[method]tcp-socket.get-is-listening",
        HostTcpSocket::get_is_listening, super::validate_tcp,
        (socket: Resource<TcpSocket>) -> bool);
    gate_socket_value!(linker, "[method]tcp-socket.get-address-family",
        HostTcpSocket::get_address_family, super::validate_tcp,
        (socket: Resource<TcpSocket>) -> IpAddressFamily);
    gate_socket!(linker, "[method]tcp-socket.set-listen-backlog-size",
        HostTcpSocket::set_listen_backlog_size, view_sync, super::validate_tcp,
        (socket: Resource<TcpSocket>, value: u64) -> ());
    tcp_options!(linker,
        "[method]tcp-socket.get-keep-alive-enabled", HostTcpSocket::get_keep_alive_enabled, bool,
        "[method]tcp-socket.set-keep-alive-enabled", HostTcpSocket::set_keep_alive_enabled, bool;
        "[method]tcp-socket.get-keep-alive-idle-time", HostTcpSocket::get_keep_alive_idle_time, u64,
        "[method]tcp-socket.set-keep-alive-idle-time", HostTcpSocket::set_keep_alive_idle_time, u64;
        "[method]tcp-socket.get-keep-alive-interval", HostTcpSocket::get_keep_alive_interval, u64,
        "[method]tcp-socket.set-keep-alive-interval", HostTcpSocket::set_keep_alive_interval, u64;
        "[method]tcp-socket.get-keep-alive-count", HostTcpSocket::get_keep_alive_count, u32,
        "[method]tcp-socket.set-keep-alive-count", HostTcpSocket::set_keep_alive_count, u32;
        "[method]tcp-socket.get-hop-limit", HostTcpSocket::get_hop_limit, u8,
        "[method]tcp-socket.set-hop-limit", HostTcpSocket::set_hop_limit, u8;
        "[method]tcp-socket.get-receive-buffer-size", HostTcpSocket::get_receive_buffer_size, u64,
        "[method]tcp-socket.set-receive-buffer-size", HostTcpSocket::set_receive_buffer_size, u64;
        "[method]tcp-socket.get-send-buffer-size", HostTcpSocket::get_send_buffer_size, u64,
        "[method]tcp-socket.set-send-buffer-size", HostTcpSocket::set_send_buffer_size, u64;
    );
    Ok(())
}
