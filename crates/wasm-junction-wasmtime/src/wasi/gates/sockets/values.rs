use wasmtime_wasi::p2::bindings::sockets::network::{
    ErrorCode, IpAddress, IpAddressFamily, IpSocketAddress, Ipv4SocketAddress, Ipv6SocketAddress,
};
use wasmtime_wasi::p2::bindings::sockets::tcp::ShutdownType;
use wasmtime_wasi::p2::bindings::sockets::udp::{IncomingDatagram, OutgoingDatagram};

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

impl ToVal for Ipv4SocketAddress {
    fn to_val(self) -> Val {
        let address = Val::Tuple(
            [
                self.address.0,
                self.address.1,
                self.address.2,
                self.address.3,
            ]
            .into_iter()
            .map(Val::U8)
            .collect(),
        );
        Val::Record(vec![
            ("port".to_owned(), self.port.to_val()),
            ("address".to_owned(), address),
        ])
    }
}

impl FromVal for Ipv4SocketAddress {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("ipv4-socket-address"));
        };
        let [(_, port), (_, address)] =
            <[_; 2]>::try_from(fields).map_err(|_| shape("ipv4-socket-address fields"))?;
        let IpAddress::Ipv4(address) = IpAddress::from_val(Val::Variant {
            case: "ipv4".to_owned(),
            value: Some(Box::new(address)),
        })?
        else {
            return Err(shape("ipv4-address"));
        };
        Ok(Self {
            port: u16::from_val(port)?,
            address,
        })
    }
}

impl ToVal for Ipv6SocketAddress {
    fn to_val(self) -> Val {
        let address = Val::Tuple(
            [
                self.address.0,
                self.address.1,
                self.address.2,
                self.address.3,
                self.address.4,
                self.address.5,
                self.address.6,
                self.address.7,
            ]
            .into_iter()
            .map(Val::U16)
            .collect(),
        );
        Val::Record(vec![
            ("port".to_owned(), self.port.to_val()),
            ("flow-info".to_owned(), self.flow_info.to_val()),
            ("address".to_owned(), address),
            ("scope-id".to_owned(), self.scope_id.to_val()),
        ])
    }
}

impl FromVal for Ipv6SocketAddress {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("ipv6-socket-address"));
        };
        let [(_, port), (_, flow_info), (_, address), (_, scope_id)] =
            <[_; 4]>::try_from(fields).map_err(|_| shape("ipv6-socket-address fields"))?;
        let IpAddress::Ipv6(address) = IpAddress::from_val(Val::Variant {
            case: "ipv6".to_owned(),
            value: Some(Box::new(address)),
        })?
        else {
            return Err(shape("ipv6-address"));
        };
        Ok(Self {
            port: u16::from_val(port)?,
            flow_info: u32::from_val(flow_info)?,
            address,
            scope_id: u32::from_val(scope_id)?,
        })
    }
}

impl ToVal for IpSocketAddress {
    fn to_val(self) -> Val {
        let (case, value) = match self {
            Self::Ipv4(value) => ("ipv4", value.to_val()),
            Self::Ipv6(value) => ("ipv6", value.to_val()),
        };
        Val::Variant {
            case: case.to_owned(),
            value: Some(Box::new(value)),
        }
    }
}

impl FromVal for IpSocketAddress {
    fn from_val(value: Val) -> Result<Self, CallError> {
        match value {
            Val::Variant {
                case,
                value: Some(value),
            } if case == "ipv4" => Ipv4SocketAddress::from_val(*value).map(Self::Ipv4),
            Val::Variant {
                case,
                value: Some(value),
            } if case == "ipv6" => Ipv6SocketAddress::from_val(*value).map(Self::Ipv6),
            _ => Err(shape("ip-socket-address")),
        }
    }
}

enum_value!(ShutdownType { Receive => "receive", Send => "send", Both => "both" });
