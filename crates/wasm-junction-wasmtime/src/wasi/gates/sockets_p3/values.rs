use std::net::{IpAddr, SocketAddr};

use wasm_junction_core::{CallError, Val};
use wasmtime::component::Resource;
use wasmtime_wasi::p3::bindings::sockets::{ip_name_lookup, types};

use super::{FromVal, INTERFACE, TCP_SOCKET, TcpSocket, ToVal, shape};
use crate::engine::StoreData;

macro_rules! variant_value {
    ($ty:ty { $($variant:ident => $name:literal),+; $other:ident => $other_name:literal }) => {
        impl ToVal for $ty {
            fn to_val(self) -> Val {
                let (case, value) = match self {
                    $(Self::$variant => ($name, None),)+
                    Self::$other(value) => ($other_name, Some(Box::new(value.to_val()))),
                };
                Val::Variant { case: case.to_owned(), value }
            }
        }

        impl FromVal for $ty {
            fn from_val(value: Val) -> Result<Self, CallError> {
                let Val::Variant { case, value } = value else { return Err(shape("variant")); };
                match (case.as_str(), value) {
                    $(($name, None) => Ok(Self::$variant),)+
                    ($other_name, Some(value)) => Option::<String>::from_val(*value).map(Self::$other),
                    _ => Err(shape(stringify!($ty))),
                }
            }
        }
    };
}

variant_value!(types::ErrorCode {
    AccessDenied => "access-denied", NotSupported => "not-supported",
    InvalidArgument => "invalid-argument", OutOfMemory => "out-of-memory",
    Timeout => "timeout", InvalidState => "invalid-state",
    AddressNotBindable => "address-not-bindable", AddressInUse => "address-in-use",
    RemoteUnreachable => "remote-unreachable", ConnectionRefused => "connection-refused",
    ConnectionBroken => "connection-broken", ConnectionReset => "connection-reset",
    ConnectionAborted => "connection-aborted", DatagramTooLarge => "datagram-too-large";
    Other => "other"
});

variant_value!(ip_name_lookup::ErrorCode {
    AccessDenied => "access-denied", InvalidArgument => "invalid-argument",
    NameUnresolvable => "name-unresolvable",
    TemporaryResolverFailure => "temporary-resolver-failure",
    PermanentResolverFailure => "permanent-resolver-failure";
    Other => "other"
});

enum_value!(types::IpAddressFamily { Ipv4 => "ipv4", Ipv6 => "ipv6" });

impl ToVal for types::IpAddress {
    fn to_val(self) -> Val {
        super::super::sockets::values::encode_ip_address(IpAddr::from(self))
    }
}

impl FromVal for types::IpAddress {
    fn from_val(value: Val) -> Result<Self, CallError> {
        super::super::sockets::values::decode_ip_address(value).map(Into::into)
    }
}

impl ToVal for types::IpSocketAddress {
    fn to_val(self) -> Val {
        super::super::sockets::values::encode_socket_address(SocketAddr::from(self))
    }
}

impl FromVal for types::IpSocketAddress {
    fn from_val(value: Val) -> Result<Self, CallError> {
        super::super::sockets::values::decode_socket_address(value).map(Into::into)
    }
}

impl crate::stream_values::StreamValue for Resource<TcpSocket> {
    fn into_val(
        self,
        _ty: Option<&wasmtime::component::Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Option<Val>, wasmtime::Error> {
        let invocation = store
            .data()
            .context
            .invocation_id()
            .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
        let id = self.rep();
        let address = types::HostTcpSocket::get_remote_address(
            &mut super::views::sockets(store.data_mut()),
            Resource::new_borrow(id),
        )
        .map_err(|error| wasmtime::Error::msg(error.to_string()))?;
        store
            .data_mut()
            .set_wasi_handle_context(id, address.to_val());
        Ok(Some(Val::Resource(
            wasm_junction_core::Resource::__owned_for_invocation(
                INTERFACE, TCP_SOCKET, id, invocation,
            ),
        )))
    }

    fn from_val(
        value: Option<Val>,
        _ty: Option<&wasmtime::component::Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Self, wasmtime::Error> {
        let Some(Val::Resource(resource)) = value else {
            return Err(wasmtime::Error::new(CallError::refused(
                "expected tcp-socket stream item",
            )));
        };
        super::validate_owned_resource::<TcpSocket>(
            &resource,
            INTERFACE,
            TCP_SOCKET,
            store.data_mut(),
        )
        .map_err(wasmtime::Error::new)?;
        Ok(Resource::new_own(resource.id()))
    }
}

list_value!(types::IpAddress);
