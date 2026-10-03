use wasm_junction_core::{CallError, Val};
use wasmtime::component::Resource;
use wasmtime_wasi::p2::bindings::sockets::{ip_name_lookup, network, tcp, udp};

use super::{WitResource, validate_borrowed, validate_handle_contexts};
use crate::engine::StoreData;

mod gate;
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
