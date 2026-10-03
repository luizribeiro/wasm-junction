use wasm_junction_core::ChannelDirection;
use wasmtime::component::Resource;
use wasmtime_wasi::p2::bindings::sockets::network::{ErrorCode, IpSocketAddress, Network};
use wasmtime_wasi::p2::bindings::sockets::tcp::{self, HostTcpSocket};
use wasmtime_wasi::p2::{DynInputStream, DynOutputStream, DynPollable, SocketError, SocketResult};

use super::super::{ToVal, open_channel, views};
use super::StoreData;

async fn start_bind(
    store: &mut StoreData,
    socket: Resource<tcp::TcpSocket>,
    network: Resource<Network>,
    address: IpSocketAddress,
) -> SocketResult<()> {
    let context = address.to_val();
    let result =
        HostTcpSocket::start_bind(&mut views::sockets(store), socket, network, address).await;
    if result.is_ok() {
        store.set_wasi_handle_context(socket.rep(), context);
    }
    result
}

fn start_connect(
    store: &mut StoreData,
    socket: Resource<tcp::TcpSocket>,
    network: Resource<Network>,
    address: IpSocketAddress,
) -> SocketResult<()> {
    let context = address.to_val();
    let result = HostTcpSocket::start_connect(&mut views::sockets(store), socket, network, address);
    if result.is_ok() {
        store.set_wasi_handle_context(socket.rep(), context);
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
    let address = HostTcpSocket::remote_address(&mut views::sockets(store), socket)?.to_val();
    for id in [socket.rep(), input.rep(), output.rep()] {
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
    let pollable = HostTcpSocket::subscribe(&mut views::sockets(store), socket)?;
    if let Some(context) = store.wasi_handle_context(socket.rep()).cloned() {
        store.set_wasi_handle_context(pollable.rep(), context);
    }
    Ok(pollable)
}
