use wasm_junction_core::{CallError, ChannelDirection};
use wasmtime::component::Linker;
use wasmtime::component::Resource;
use wasmtime_wasi::p2::bindings::sockets::network::{ErrorCode, IpSocketAddress, Network};
use wasmtime_wasi::p2::bindings::sockets::tcp::{self, HostTcpSocket, ShutdownType};
use wasmtime_wasi::p2::bindings::sockets::tcp_create_socket;
use wasmtime_wasi::p2::{DynInputStream, DynOutputStream, DynPollable, SocketError, SocketResult};

use super::super::{
    FromVal, Real, ToVal, finish, open_channel, scope_values, shape, trampoline, views,
};
use super::{StoreData, gate_socket, validate_tcp};

async fn start_bind(
    store: &mut StoreData,
    socket: Resource<tcp::TcpSocket>,
    network: Resource<Network>,
    address: IpSocketAddress,
) -> SocketResult<()> {
    let socket_id = socket.rep();
    let context = address.to_val();
    let result =
        HostTcpSocket::start_bind(&mut views::sockets(store), socket, network, address).await;
    if result.is_ok() {
        store.set_wasi_handle_context(socket_id, context);
    }
    result
}

fn start_connect(
    store: &mut StoreData,
    socket: Resource<tcp::TcpSocket>,
    network: Resource<Network>,
    address: IpSocketAddress,
) -> SocketResult<()> {
    let socket_id = socket.rep();
    let context = address.to_val();
    let result = HostTcpSocket::start_connect(&mut views::sockets(store), socket, network, address);
    if result.is_ok() {
        store.set_wasi_handle_context(socket_id, context);
    }
    result
}

fn finish_connect(
    store: &mut StoreData,
    socket: Resource<tcp::TcpSocket>,
) -> SocketResult<(Resource<DynInputStream>, Resource<DynOutputStream>)> {
    let context = store
        .wasi_handle_context(socket.rep())
        .cloned()
        .ok_or(ErrorCode::AccessDenied)?;
    let (input, output) = HostTcpSocket::finish_connect(&mut views::sockets(store), socket)?;
    store.set_wasi_handle_context(input.rep(), context.clone());
    store.set_wasi_handle_context(output.rep(), context);
    open_channel(&input, store, ChannelDirection::HostToGuest).map_err(SocketError::trap)?;
    open_channel(&output, store, ChannelDirection::GuestToHost).map_err(SocketError::trap)?;
    Ok((input, output))
}

fn accept(
    store: &mut StoreData,
    listener: Resource<tcp::TcpSocket>,
) -> SocketResult<(
    Resource<tcp::TcpSocket>,
    Resource<DynInputStream>,
    Resource<DynOutputStream>,
)> {
    let (socket, input, output) = HostTcpSocket::accept(&mut views::sockets(store), listener)?;
    let socket_id = socket.rep();
    let address =
        HostTcpSocket::remote_address(&mut views::sockets(store), Resource::new_borrow(socket_id))?
            .to_val();
    for id in [socket_id, input.rep(), output.rep()] {
        store.set_wasi_handle_context(id, address.clone());
    }
    open_channel(&input, store, ChannelDirection::HostToGuest).map_err(SocketError::trap)?;
    open_channel(&output, store, ChannelDirection::GuestToHost).map_err(SocketError::trap)?;
    Ok((socket, input, output))
}

fn subscribe(
    store: &mut StoreData,
    socket: Resource<tcp::TcpSocket>,
) -> wasmtime::Result<Resource<DynPollable>> {
    let socket_id = socket.rep();
    let pollable = HostTcpSocket::subscribe(&mut views::sockets(store), socket)?;
    if let Some(context) = store.wasi_handle_context(socket_id).cloned() {
        store.set_wasi_handle_context(pollable.rep(), context);
    }
    Ok(pollable)
}

macro_rules! tcp_options {
    ($linker:ident, $(
        $get_name:literal, $get:path, $get_ty:ty,
        $set_name:literal, $set:path, $set_ty:ty;
    )+) => {$(
        gate_socket!($linker, super::TCP_INTERFACE,
            $get_name, $get, view_sync, super::validate_tcp,
            (socket: Resource<tcp::TcpSocket>) -> $get_ty);
        gate_socket!($linker, super::TCP_INTERFACE,
            $set_name, $set, view_sync, super::validate_tcp,
            (socket: Resource<tcp::TcpSocket>, value: $set_ty) -> ());
    )+};
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.start-bind",
        start_bind, store_async, super::validate_tcp_network,
        (socket: Resource<tcp::TcpSocket>, network: Resource<Network>, address: IpSocketAddress) -> ());
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.finish-bind",
        HostTcpSocket::finish_bind, view_sync, super::validate_tcp,
        (socket: Resource<tcp::TcpSocket>) -> ());
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.start-connect",
        start_connect, store_sync, super::validate_tcp_network,
        (socket: Resource<tcp::TcpSocket>, network: Resource<Network>, address: IpSocketAddress) -> ());
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.finish-connect",
        finish_connect, store_sync, super::validate_tcp,
        (socket: Resource<tcp::TcpSocket>) -> (Resource<DynInputStream>, Resource<DynOutputStream>));
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.start-listen",
        HostTcpSocket::start_listen, view_async, super::validate_tcp,
        (socket: Resource<tcp::TcpSocket>) -> ());
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.finish-listen",
        HostTcpSocket::finish_listen, view_sync, super::validate_tcp,
        (socket: Resource<tcp::TcpSocket>) -> ());
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.accept",
        accept, store_sync, super::validate_tcp,
        (socket: Resource<tcp::TcpSocket>) -> (Resource<tcp::TcpSocket>, Resource<DynInputStream>, Resource<DynOutputStream>));
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.local-address",
        HostTcpSocket::local_address, view_sync, super::validate_tcp,
        (socket: Resource<tcp::TcpSocket>) -> IpSocketAddress);
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.remote-address",
        HostTcpSocket::remote_address, view_sync, super::validate_tcp,
        (socket: Resource<tcp::TcpSocket>) -> IpSocketAddress);
    gate!(linker, "wasi:sockets/tcp@0.2.12", "[method]tcp-socket.is-listening", sockets,
        HostTcpSocket::is_listening, plain_with[validate_tcp],
        (socket: Resource<tcp::TcpSocket>) -> bool);
    gate!(linker, "wasi:sockets/tcp@0.2.12", "[method]tcp-socket.address-family", sockets,
        HostTcpSocket::address_family, plain_with[validate_tcp],
        (socket: Resource<tcp::TcpSocket>) -> tcp::IpAddressFamily);
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.set-listen-backlog-size",
        HostTcpSocket::set_listen_backlog_size, view_sync, super::validate_tcp,
        (socket: Resource<tcp::TcpSocket>, value: u64) -> ());
    tcp_options!(linker,
        "[method]tcp-socket.keep-alive-enabled", HostTcpSocket::keep_alive_enabled, bool,
        "[method]tcp-socket.set-keep-alive-enabled", HostTcpSocket::set_keep_alive_enabled, bool;
        "[method]tcp-socket.keep-alive-idle-time", HostTcpSocket::keep_alive_idle_time, u64,
        "[method]tcp-socket.set-keep-alive-idle-time", HostTcpSocket::set_keep_alive_idle_time, u64;
        "[method]tcp-socket.keep-alive-interval", HostTcpSocket::keep_alive_interval, u64,
        "[method]tcp-socket.set-keep-alive-interval", HostTcpSocket::set_keep_alive_interval, u64;
        "[method]tcp-socket.keep-alive-count", HostTcpSocket::keep_alive_count, u32,
        "[method]tcp-socket.set-keep-alive-count", HostTcpSocket::set_keep_alive_count, u32;
        "[method]tcp-socket.hop-limit", HostTcpSocket::hop_limit, u8,
        "[method]tcp-socket.set-hop-limit", HostTcpSocket::set_hop_limit, u8;
        "[method]tcp-socket.receive-buffer-size", HostTcpSocket::receive_buffer_size, u64,
        "[method]tcp-socket.set-receive-buffer-size", HostTcpSocket::set_receive_buffer_size, u64;
        "[method]tcp-socket.send-buffer-size", HostTcpSocket::send_buffer_size, u64,
        "[method]tcp-socket.set-send-buffer-size", HostTcpSocket::set_send_buffer_size, u64;
    );
    gate!(linker, "wasi:sockets/tcp@0.2.12", "[method]tcp-socket.subscribe", store,
        subscribe, plain_with[validate_tcp],
        (socket: Resource<tcp::TcpSocket>) -> Resource<DynPollable>);
    gate_socket!(linker, super::TCP_INTERFACE, "[method]tcp-socket.shutdown",
        HostTcpSocket::shutdown, view_sync, super::validate_tcp,
        (socket: Resource<tcp::TcpSocket>, kind: ShutdownType) -> ());
    gate_socket!(linker, "wasi:sockets/tcp-create-socket@0.2.12", "create-tcp-socket",
        tcp_create_socket::Host::create_tcp_socket, view_sync, super::validate_none,
        (family: tcp::IpAddressFamily) -> Resource<tcp::TcpSocket>);
    Ok(())
}
