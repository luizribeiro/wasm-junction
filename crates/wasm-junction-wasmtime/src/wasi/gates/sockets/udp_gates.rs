use wasm_junction_core::{CallError, ChannelDirection};
use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::p2::bindings::sockets::network::{ErrorCode, IpSocketAddress, Network};
use wasmtime_wasi::p2::bindings::sockets::udp::{self, HostUdpSocket};
use wasmtime_wasi::p2::bindings::sockets::udp_create_socket;
use wasmtime_wasi::p2::{DynPollable, SocketError, SocketResult};

use super::super::{
    FromVal, Real, ToVal, finish, open_channel, scope_values, shape, trampoline, views,
};
use super::gate::socket_options;
use super::{
    StoreData, copy_context, gate_socket, validate_incoming, validate_outgoing, validate_udp,
};

async fn start_bind(
    store: &mut StoreData,
    socket: Resource<udp::UdpSocket>,
    network: Resource<Network>,
    address: IpSocketAddress,
) -> SocketResult<()> {
    let socket_id = socket.rep();
    let context = address.to_val();
    let result =
        HostUdpSocket::start_bind(&mut views::sockets(store), socket, network, address).await;
    if result.is_ok() {
        store.set_socket_local_address(socket_id, context);
    }
    result
}

async fn stream(
    store: &mut StoreData,
    socket: Resource<udp::UdpSocket>,
    remote: Option<IpSocketAddress>,
) -> SocketResult<(
    Resource<udp::IncomingDatagramStream>,
    Resource<udp::OutgoingDatagramStream>,
)> {
    let socket_id = socket.rep();
    let remote_context = remote.map(ToVal::to_val);
    let streams = HostUdpSocket::stream(&mut views::sockets(store), socket, remote).await?;
    if let Some(context) = remote_context {
        store.set_wasi_handle_context(socket_id, context);
    } else if !store.use_socket_local_address(socket_id) {
        return Err(ErrorCode::AccessDenied.into());
    }
    let context = store
        .wasi_handle_context(socket_id)
        .cloned()
        .ok_or(ErrorCode::AccessDenied)?;
    store.set_wasi_handle_context(streams.0.rep(), context.clone());
    store.set_wasi_handle_context(streams.1.rep(), context);
    open_channel(&streams.0, store, ChannelDirection::HostToGuest).map_err(SocketError::trap)?;
    open_channel(&streams.1, store, ChannelDirection::GuestToHost).map_err(SocketError::trap)?;
    Ok(streams)
}

fn subscribe_socket(
    store: &mut StoreData,
    socket: Resource<udp::UdpSocket>,
) -> wasmtime::Result<Resource<DynPollable>> {
    let socket_id = socket.rep();
    let pollable = HostUdpSocket::subscribe(&mut views::sockets(store), socket)?;
    copy_context(store, socket_id, pollable.rep());
    Ok(pollable)
}

fn subscribe_incoming(
    store: &mut StoreData,
    stream: Resource<udp::IncomingDatagramStream>,
) -> wasmtime::Result<Resource<DynPollable>> {
    let stream_id = stream.rep();
    let pollable = udp::HostIncomingDatagramStream::subscribe(&mut views::sockets(store), stream)?;
    copy_context(store, stream_id, pollable.rep());
    Ok(pollable)
}

fn subscribe_outgoing(
    store: &mut StoreData,
    stream: Resource<udp::OutgoingDatagramStream>,
) -> wasmtime::Result<Resource<DynPollable>> {
    let stream_id = stream.rep();
    let pollable = udp::HostOutgoingDatagramStream::subscribe(&mut views::sockets(store), stream)?;
    copy_context(store, stream_id, pollable.rep());
    Ok(pollable)
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_socket!(linker, super::UDP_INTERFACE, "[method]udp-socket.start-bind", start_bind,
        store_async, super::validate_udp_network,
        (socket: Resource<udp::UdpSocket>, network: Resource<Network>, address: IpSocketAddress) -> ());
    gate_socket!(linker, super::UDP_INTERFACE, "[method]udp-socket.finish-bind",
        HostUdpSocket::finish_bind, view_sync, super::validate_udp,
        (socket: Resource<udp::UdpSocket>) -> ());
    gate_socket!(linker, super::UDP_INTERFACE, "[method]udp-socket.stream", stream,
        store_async, super::validate_udp,
        (socket: Resource<udp::UdpSocket>, remote: Option<IpSocketAddress>) ->
            (Resource<udp::IncomingDatagramStream>, Resource<udp::OutgoingDatagramStream>));
    gate_socket!(linker, super::UDP_INTERFACE, "[method]udp-socket.local-address",
        HostUdpSocket::local_address, view_sync, super::validate_udp,
        (socket: Resource<udp::UdpSocket>) -> IpSocketAddress);
    gate_socket!(linker, super::UDP_INTERFACE, "[method]udp-socket.remote-address",
        HostUdpSocket::remote_address, view_sync, super::validate_udp,
        (socket: Resource<udp::UdpSocket>) -> IpSocketAddress);
    gate!(linker, "wasi:sockets/udp@0.2.12", "[method]udp-socket.address-family", sockets,
        HostUdpSocket::address_family, plain_with[validate_udp],
        (socket: Resource<udp::UdpSocket>) -> udp::IpAddressFamily);
    socket_options!(gate_socket, linker, super::UDP_INTERFACE, udp::UdpSocket,
        super::validate_udp,
        "[method]udp-socket.unicast-hop-limit", HostUdpSocket::unicast_hop_limit, u8,
        "[method]udp-socket.set-unicast-hop-limit", HostUdpSocket::set_unicast_hop_limit, u8;
        "[method]udp-socket.receive-buffer-size", HostUdpSocket::receive_buffer_size, u64,
        "[method]udp-socket.set-receive-buffer-size", HostUdpSocket::set_receive_buffer_size, u64;
        "[method]udp-socket.send-buffer-size", HostUdpSocket::send_buffer_size, u64,
        "[method]udp-socket.set-send-buffer-size", HostUdpSocket::set_send_buffer_size, u64;
    );
    gate!(linker, "wasi:sockets/udp@0.2.12", "[method]udp-socket.subscribe", store,
        subscribe_socket, plain_with[validate_udp],
        (socket: Resource<udp::UdpSocket>) -> Resource<DynPollable>);
    gate_socket!(linker, super::UDP_INTERFACE, "[method]incoming-datagram-stream.receive",
        udp::HostIncomingDatagramStream::receive, view_sync, super::validate_incoming,
        (stream: Resource<udp::IncomingDatagramStream>, max_results: u64) -> Vec<udp::IncomingDatagram>);
    gate!(linker, "wasi:sockets/udp@0.2.12", "[method]incoming-datagram-stream.subscribe", store,
        subscribe_incoming, plain_with[validate_incoming],
        (stream: Resource<udp::IncomingDatagramStream>) -> Resource<DynPollable>);
    gate_socket!(linker, super::UDP_INTERFACE, "[method]outgoing-datagram-stream.check-send",
        udp::HostOutgoingDatagramStream::check_send, view_sync, super::validate_outgoing,
        (stream: Resource<udp::OutgoingDatagramStream>) -> u64);
    gate_socket!(linker, super::UDP_INTERFACE, "[method]outgoing-datagram-stream.send",
        udp::HostOutgoingDatagramStream::send, view_sync, super::validate_outgoing,
        (stream: Resource<udp::OutgoingDatagramStream>, datagrams: Vec<udp::OutgoingDatagram>) -> u64);
    gate!(linker, "wasi:sockets/udp@0.2.12", "[method]outgoing-datagram-stream.subscribe", store,
        subscribe_outgoing, plain_with[validate_outgoing],
        (stream: Resource<udp::OutgoingDatagramStream>) -> Resource<DynPollable>);
    gate_socket!(linker, "wasi:sockets/udp-create-socket@0.2.12", "create-udp-socket",
        udp_create_socket::Host::create_udp_socket, view_async, super::validate_none,
        (family: udp::IpAddressFamily) -> Resource<udp::UdpSocket>);
    Ok(())
}
