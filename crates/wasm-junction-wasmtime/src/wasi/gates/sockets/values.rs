use wasmtime_wasi::p2::bindings::sockets::network::{
    ErrorCode, IpAddress, IpAddressFamily, IpSocketAddress, Ipv4SocketAddress, Ipv6SocketAddress,
};

use super::super::{FromVal, ToVal, shape};
use wasm_junction_core::{CallError, Val};

macro_rules! enum_value {
    ($ty:ty { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl ToVal for $ty {
            fn to_val(self) -> Val {
                Val::Enum(match self { $(Self::$variant => $name),+ }.to_owned())
            }
        }

        impl FromVal for $ty {
            fn from_val(value: Val) -> Result<Self, CallError> {
                let Val::Enum(value) = value else { return Err(shape("enum")); };
                match value.as_str() {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(shape(stringify!($ty))),
                }
            }
        }
    };
}

enum_value!(ErrorCode {
    Unknown => "unknown",
    AccessDenied => "access-denied",
    NotSupported => "not-supported",
    InvalidArgument => "invalid-argument",
    OutOfMemory => "out-of-memory",
    Timeout => "timeout",
    ConcurrencyConflict => "concurrency-conflict",
    NotInProgress => "not-in-progress",
    WouldBlock => "would-block",
    InvalidState => "invalid-state",
    NewSocketLimit => "new-socket-limit",
    AddressNotBindable => "address-not-bindable",
    AddressInUse => "address-in-use",
    RemoteUnreachable => "remote-unreachable",
    ConnectionRefused => "connection-refused",
    ConnectionReset => "connection-reset",
    ConnectionAborted => "connection-aborted",
    DatagramTooLarge => "datagram-too-large",
    NameUnresolvable => "name-unresolvable",
    TemporaryResolverFailure => "temporary-resolver-failure",
    PermanentResolverFailure => "permanent-resolver-failure",
});

enum_value!(IpAddressFamily { Ipv4 => "ipv4", Ipv6 => "ipv6" });

impl ToVal for IpAddress {
    fn to_val(self) -> Val {
        let (case, value) = match self {
            Self::Ipv4(address) => (
                "ipv4",
                vec![address.0, address.1, address.2, address.3]
                    .into_iter()
                    .map(Val::U8)
                    .collect(),
            ),
            Self::Ipv6(address) => (
                "ipv6",
                vec![
                    address.0, address.1, address.2, address.3, address.4, address.5, address.6,
                    address.7,
                ]
                .into_iter()
                .map(Val::U16)
                .collect(),
            ),
        };
        Val::Variant {
            case: case.to_owned(),
            value: Some(Box::new(Val::Tuple(value))),
        }
    }
}

impl FromVal for IpAddress {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Variant {
            case,
            value: Some(value),
        } = value
        else {
            return Err(shape("ip-address"));
        };
        let Val::Tuple(fields) = *value else {
            return Err(shape("ip-address tuple"));
        };
        match case.as_str() {
            "ipv4" => {
                let [a, b, c, d] = <[Val; 4]>::try_from(fields).map_err(|_| shape("ipv4"))?;
                Ok(Self::Ipv4((
                    u8::from_val(a)?,
                    u8::from_val(b)?,
                    u8::from_val(c)?,
                    u8::from_val(d)?,
                )))
            }
            "ipv6" => {
                let [a, b, c, d, e, f, g, h] =
                    <[Val; 8]>::try_from(fields).map_err(|_| shape("ipv6"))?;
                Ok(Self::Ipv6((
                    u16::from_val(a)?,
                    u16::from_val(b)?,
                    u16::from_val(c)?,
                    u16::from_val(d)?,
                    u16::from_val(e)?,
                    u16::from_val(f)?,
                    u16::from_val(g)?,
                    u16::from_val(h)?,
                )))
            }
            _ => Err(shape("ip-address case")),
        }
    }
}
