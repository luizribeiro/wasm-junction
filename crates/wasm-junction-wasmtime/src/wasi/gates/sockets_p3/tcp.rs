use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::p3::bindings::sockets::types::{
    HostTcpSocket, HostTcpSocketWithStore, IpAddressFamily, IpSocketAddress, TcpSocket,
};
use wasmtime_wasi::p3::sockets::SocketResult;
use wasmtime_wasi::sockets::{WasiSockets, WasiSocketsView};

use super::gate::{gate_socket, gate_socket_concurrent, gate_socket_value};
#[allow(
    clippy::wildcard_imports,
    reason = "TCP gates share the socket module's private gate machinery"
)]
use super::*;

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

fn drop_socket(store: &mut StoreData, socket: Resource<TcpSocket>) -> wasmtime::Result<()> {
    HostTcpSocket::drop(&mut views::sockets(store), socket)
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
