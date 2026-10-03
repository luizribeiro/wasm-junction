use wasm_junction_core::{CallError, Val, WasiSettings};
use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::p2::bindings::sockets::{ip_name_lookup, network, tcp, udp};
use wasmtime_wasi::p2::{DynPollable, IoError, SocketResult};

use super::{
    ToVal, WitResource, validate_borrowed, validate_error_borrow, validate_handle_contexts, views,
};
use crate::engine::StoreData;

mod gate;
mod tcp_gates;
mod udp_gates;
mod values;

use gate::gate_socket;

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
    let pollable =
        ip_name_lookup::HostResolveAddressStream::subscribe(&mut views::sockets(store), stream)?;
    if let Some(context) = store.wasi_handle_context(stream.rep()).cloned() {
        store.set_wasi_handle_context(pollable.rep(), context);
    }
    Ok(pollable)
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
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
