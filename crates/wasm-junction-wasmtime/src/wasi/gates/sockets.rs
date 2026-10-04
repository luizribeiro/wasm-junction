use wasm_junction_core::{CallError, EngineEvent, Resource as JunctionResource, Val, WasiSettings};
use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::p2::bindings::sockets::{ip_name_lookup, network, tcp, udp};
use wasmtime_wasi::p2::{DynPollable, IoError, SocketResult};

use super::{
    FromVal, Real, ToVal, WitResource, add_handle_contexts, close_channel, finish,
    no_resource_validation, scope_values, shape, trampoline, validate_borrowed,
    validate_error_borrow, validate_handle_contexts, validate_owned, views,
};
use crate::engine::StoreData;

mod gate;
mod tcp_gates;
mod udp_gates;
mod values;

use gate::{copy_context, gate_socket};

const NETWORK_INTERFACE: &str = "wasi:sockets/network@0.2.12";
const LOOKUP_INTERFACE: &str = "wasi:sockets/ip-name-lookup@0.2.12";
const TCP_INTERFACE: &str = "wasi:sockets/tcp@0.2.12";
const UDP_INTERFACE: &str = "wasi:sockets/udp@0.2.12";

const NETWORK: &str = "network";
const RESOLVE_STREAM: &str = "resolve-address-stream";
const TCP_SOCKET: &str = "tcp-socket";
const UDP_SOCKET: &str = "udp-socket";
const INCOMING_DATAGRAM_STREAM: &str = "incoming-datagram-stream";
const OUTGOING_DATAGRAM_STREAM: &str = "outgoing-datagram-stream";

impl WitResource for network::Network {
    const INTERFACE: &'static str = NETWORK_INTERFACE;
    const NAME: &'static str = NETWORK;
}

impl WitResource for ip_name_lookup::ResolveAddressStream {
    const INTERFACE: &'static str = LOOKUP_INTERFACE;
    const NAME: &'static str = RESOLVE_STREAM;
}

impl WitResource for tcp::TcpSocket {
    const INTERFACE: &'static str = TCP_INTERFACE;
    const NAME: &'static str = TCP_SOCKET;
}

impl WitResource for udp::UdpSocket {
    const INTERFACE: &'static str = UDP_INTERFACE;
    const NAME: &'static str = UDP_SOCKET;
}

impl WitResource for udp::IncomingDatagramStream {
    const INTERFACE: &'static str = UDP_INTERFACE;
    const NAME: &'static str = INCOMING_DATAGRAM_STREAM;
}

impl WitResource for udp::OutgoingDatagramStream {
    const INTERFACE: &'static str = UDP_INTERFACE;
    const NAME: &'static str = OUTGOING_DATAGRAM_STREAM;
}

fn validate_at<T: WitResource>(
    values: &[Val],
    position: usize,
    store: &mut StoreData,
) -> Result<(), CallError> {
    validate_borrowed::<T>(
        values.get(position).ok_or_else(|| super::shape(T::NAME))?,
        store,
    )
}

fn validate_none(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_handle_contexts(values, store)
}

macro_rules! validator {
    ($name:ident, $($position:literal => $ty:ty),+ $(,)?) => {
        fn $name(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
            $(validate_at::<$ty>(values, $position, store)?;)+
            validate_handle_contexts(values, store)
        }
    };
}

validator!(validate_network, 0 => network::Network);
validator!(validate_resolve_stream, 0 => ip_name_lookup::ResolveAddressStream);
validator!(validate_tcp, 0 => tcp::TcpSocket);
validator!(validate_tcp_network,
    0 => tcp::TcpSocket,
    1 => network::Network,
);
validator!(validate_udp, 0 => udp::UdpSocket);
validator!(validate_udp_network,
    0 => udp::UdpSocket,
    1 => network::Network,
);
validator!(validate_incoming, 0 => udp::IncomingDatagramStream);
validator!(validate_outgoing, 0 => udp::OutgoingDatagramStream);

fn resolve_addresses(
    store: &mut StoreData,
    network: Resource<network::Network>,
    name: String,
) -> SocketResult<Resource<ip_name_lookup::ResolveAddressStream>> {
    // Wasmtime reports disabled DNS as permanent-resolver-failure; the gate promises access-denied.
    let enabled = store
        .context
        .settings()
        .get::<WasiSettings>()
        .is_some_and(WasiSettings::sockets_enabled);
    if !enabled {
        return Err(network::ErrorCode::AccessDenied.into());
    }
    let context = name.clone().to_val();
    let stream =
        ip_name_lookup::Host::resolve_addresses(&mut views::sockets(store), network, name)?;
    store.set_wasi_handle_context(stream.rep(), context);
    Ok(stream)
}

fn subscribe_resolve_stream(
    store: &mut StoreData,
    stream: Resource<ip_name_lookup::ResolveAddressStream>,
) -> wasmtime::Result<Resource<DynPollable>> {
    let stream_id = stream.rep();
    let pollable =
        ip_name_lookup::HostResolveAddressStream::subscribe(&mut views::sockets(store), stream)?;
    if let Some(context) = store.wasi_handle_context(stream_id).cloned() {
        store.set_wasi_handle_context(pollable.rep(), context);
    }
    Ok(pollable)
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    add_drops(linker)?;
    gate!(linker, "wasi:sockets/network@0.2.12", "network-error-code", sockets,
        network::Host::network_error_code, plain_with[validate_error_borrow],
        (error: Resource<IoError>) -> Option<network::ErrorCode>);
    gate!(linker, "wasi:sockets/instance-network@0.2.12", "instance-network", sockets,
        wasmtime_wasi::p2::bindings::sockets::instance_network::Host::instance_network,
        resource, () -> Resource<network::Network>);
    gate_socket!(linker, LOOKUP_INTERFACE, "resolve-addresses", resolve_addresses,
        store_sync, validate_network,
        (network: Resource<network::Network>, name: String) ->
            Resource<ip_name_lookup::ResolveAddressStream>);
    gate_socket!(linker, LOOKUP_INTERFACE,
        "[method]resolve-address-stream.resolve-next-address",
        ip_name_lookup::HostResolveAddressStream::resolve_next_address,
        view_sync, validate_resolve_stream,
        (stream: Resource<ip_name_lookup::ResolveAddressStream>) -> Option<network::IpAddress>);
    gate!(linker, "wasi:sockets/ip-name-lookup@0.2.12",
        "[method]resolve-address-stream.subscribe", store, subscribe_resolve_stream,
        plain_with[validate_resolve_stream],
        (stream: Resource<ip_name_lookup::ResolveAddressStream>) -> Resource<DynPollable>);
    tcp_gates::add(linker)?;
    udp_gates::add(linker)?;
    Ok(())
}

fn add_drops(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_drop!(
        linker,
        NETWORK_INTERFACE,
        NETWORK,
        "[drop]network",
        network::Network,
        store,
        None,
        drop_network
    );
    gate_drop!(
        linker,
        LOOKUP_INTERFACE,
        RESOLVE_STREAM,
        "[drop]resolve-address-stream",
        ip_name_lookup::ResolveAddressStream,
        store,
        None,
        drop_resolve_stream,
        await
    );
    gate_drop!(
        linker,
        TCP_INTERFACE,
        TCP_SOCKET,
        "[drop]tcp-socket",
        tcp::TcpSocket,
        store,
        None,
        drop_tcp_socket
    );
    gate_drop!(
        linker,
        UDP_INTERFACE,
        UDP_SOCKET,
        "[drop]udp-socket",
        udp::UdpSocket,
        store,
        None,
        drop_udp_socket
    );
    gate_drop!(
        linker,
        UDP_INTERFACE,
        INCOMING_DATAGRAM_STREAM,
        "[drop]incoming-datagram-stream",
        udp::IncomingDatagramStream,
        store,
        Some(wasm_junction_core::ChannelDirection::HostToGuest),
        drop_incoming_datagram_stream,
        await
    );
    gate_drop!(
        linker,
        UDP_INTERFACE,
        OUTGOING_DATAGRAM_STREAM,
        "[drop]outgoing-datagram-stream",
        udp::OutgoingDatagramStream,
        store,
        Some(wasm_junction_core::ChannelDirection::GuestToHost),
        drop_outgoing_datagram_stream,
        await
    );
    Ok(())
}

fn drop_network(
    store: &mut StoreData,
    resource: Resource<network::Network>,
) -> wasmtime::Result<()> {
    network::HostNetwork::drop(&mut views::sockets(store), resource)
}

async fn drop_resolve_stream(
    store: &mut StoreData,
    resource: Resource<ip_name_lookup::ResolveAddressStream>,
) -> wasmtime::Result<()> {
    ip_name_lookup::HostResolveAddressStream::drop(&mut views::sockets(store), resource).await
}

fn drop_tcp_socket(
    store: &mut StoreData,
    resource: Resource<tcp::TcpSocket>,
) -> wasmtime::Result<()> {
    tcp::HostTcpSocket::drop(&mut views::sockets(store), resource)
}

fn drop_udp_socket(
    store: &mut StoreData,
    resource: Resource<udp::UdpSocket>,
) -> wasmtime::Result<()> {
    udp::HostUdpSocket::drop(&mut views::sockets(store), resource)
}

async fn drop_incoming_datagram_stream(
    store: &mut StoreData,
    resource: Resource<udp::IncomingDatagramStream>,
) -> wasmtime::Result<()> {
    udp::HostIncomingDatagramStream::drop(&mut views::sockets(store), resource).await
}

async fn drop_outgoing_datagram_stream(
    store: &mut StoreData,
    resource: Resource<udp::OutgoingDatagramStream>,
) -> wasmtime::Result<()> {
    udp::HostOutgoingDatagramStream::drop(&mut views::sockets(store), resource).await
}
