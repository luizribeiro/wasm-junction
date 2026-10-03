use wasm_junction_core::ChannelDirection;
use wasmtime::component::Resource;
use wasmtime_wasi::p2::bindings::sockets::network::{ErrorCode, IpSocketAddress, Network};
use wasmtime_wasi::p2::bindings::sockets::udp::{self, HostUdpSocket};
use wasmtime_wasi::p2::{DynPollable, SocketError, SocketResult};

use super::super::{ToVal, open_channel, views};
use super::StoreData;

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

fn copy_context(store: &mut StoreData, source: u32, target: u32) {
    if let Some(context) = store.wasi_handle_context(source).cloned() {
        store.set_wasi_handle_context(target, context);
    }
}
