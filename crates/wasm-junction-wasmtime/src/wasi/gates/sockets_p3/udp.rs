use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::p3::bindings::sockets::types::{
    HostUdpSocket, HostUdpSocketWithStore, IpAddressFamily, IpSocketAddress, UdpSocket,
};
use wasmtime_wasi::p3::sockets::{SocketError, SocketResult};
use wasmtime_wasi::sockets::{WasiSockets, WasiSocketsView};

use super::gate::{gate_socket, gate_socket_concurrent, gate_socket_value};
#[allow(
    clippy::wildcard_imports,
    reason = "UDP gates share the socket module's private gate machinery"
)]
use super::*;

async fn bind(
    store: &mut StoreData,
    socket: Resource<UdpSocket>,
    address: IpSocketAddress,
) -> SocketResult<()> {
    require_sockets(store).map_err(SocketError::trap)?;
    let id = socket.rep();
    HostUdpSocket::bind(&mut views::sockets(store), socket, address).await?;
    let actual =
        HostUdpSocket::get_local_address(&mut views::sockets(store), Resource::new_borrow(id))?;
    store.set_socket_local_address(id, actual.to_val());
    Ok(())
}

async fn connect(
    store: &mut StoreData,
    socket: Resource<UdpSocket>,
    address: IpSocketAddress,
) -> SocketResult<()> {
    require_sockets(store).map_err(SocketError::trap)?;
    let id = socket.rep();
    HostUdpSocket::connect(&mut views::sockets(store), socket, address).await?;
    store.set_wasi_handle_context(id, address.to_val());
    Ok(())
}

async fn send(
    accessor: &wasmtime::component::Accessor<StoreData, WasiSockets>,
    socket: Resource<UdpSocket>,
    data: Vec<u8>,
    remote: Option<IpSocketAddress>,
) -> SocketResult<()> {
    accessor
        .with(|mut access| require_sockets(access.as_context_mut().data()))
        .map_err(SocketError::trap)?;
    HostUdpSocketWithStore::send(accessor, socket, data, remote).await
}

async fn receive(
    accessor: &wasmtime::component::Accessor<StoreData, WasiSockets>,
    socket: Resource<UdpSocket>,
) -> SocketResult<(Vec<u8>, IpSocketAddress)> {
    accessor
        .with(|mut access| require_sockets(access.as_context_mut().data()))
        .map_err(SocketError::trap)?;
    HostUdpSocketWithStore::receive(accessor, socket).await
}

fn drop_socket(store: &mut StoreData, socket: Resource<UdpSocket>) -> wasmtime::Result<()> {
    HostUdpSocket::drop(&mut views::sockets(store), socket)
}

macro_rules! udp_options {
    ($linker:ident, $($get:literal, $get_method:path, $get_ty:ty,
        $set:literal, $set_method:path, $set_ty:ty;)+) => {$(
        gate_socket!($linker, $get, $get_method, view_sync, super::validate_udp,
            (socket: Resource<UdpSocket>) -> $get_ty);
        gate_socket!($linker, $set, $set_method, view_sync, super::validate_udp,
            (socket: Resource<UdpSocket>, value: $set_ty) -> ());
    )+};
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_drop!(
        linker,
        INTERFACE,
        UDP_SOCKET,
        "[drop]udp-socket",
        UdpSocket,
        store,
        None,
        drop_socket
    );
    gate_socket!(linker, "[static]udp-socket.create", HostUdpSocket::create, view_async,
        super::validate_none, (family: IpAddressFamily) -> Resource<UdpSocket>);
    gate_socket!(linker, "[method]udp-socket.bind", bind, store_async, super::validate_udp,
        (socket: Resource<UdpSocket>, address: IpSocketAddress) -> ());
    gate_socket!(linker, "[method]udp-socket.connect", connect, store_async, super::validate_udp,
        (socket: Resource<UdpSocket>, address: IpSocketAddress) -> ());
    gate_socket!(linker, "[method]udp-socket.disconnect", HostUdpSocket::disconnect, view_sync,
        super::validate_udp, (socket: Resource<UdpSocket>) -> ());
    gate_socket_concurrent!(linker, "[method]udp-socket.send", send, super::validate_udp,
        (socket: Resource<UdpSocket>, data: Vec<u8>, remote: Option<IpSocketAddress>) -> ());
    gate_socket_concurrent!(linker, "[method]udp-socket.receive", receive, super::validate_udp,
        (socket: Resource<UdpSocket>) -> (Vec<u8>, IpSocketAddress));
    gate_socket!(linker, "[method]udp-socket.get-local-address",
        HostUdpSocket::get_local_address, view_sync, super::validate_udp,
        (socket: Resource<UdpSocket>) -> IpSocketAddress);
    gate_socket!(linker, "[method]udp-socket.get-remote-address",
        HostUdpSocket::get_remote_address, view_sync, super::validate_udp,
        (socket: Resource<UdpSocket>) -> IpSocketAddress);
    gate_socket_value!(linker, "[method]udp-socket.get-address-family",
        HostUdpSocket::get_address_family, super::validate_udp,
        (socket: Resource<UdpSocket>) -> IpAddressFamily);
    udp_options!(linker,
        "[method]udp-socket.get-unicast-hop-limit", HostUdpSocket::get_unicast_hop_limit, u8,
        "[method]udp-socket.set-unicast-hop-limit", HostUdpSocket::set_unicast_hop_limit, u8;
        "[method]udp-socket.get-receive-buffer-size", HostUdpSocket::get_receive_buffer_size, u64,
        "[method]udp-socket.set-receive-buffer-size", HostUdpSocket::set_receive_buffer_size, u64;
        "[method]udp-socket.get-send-buffer-size", HostUdpSocket::get_send_buffer_size, u64,
        "[method]udp-socket.set-send-buffer-size", HostUdpSocket::set_send_buffer_size, u64;
    );
    Ok(())
}
