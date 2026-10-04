use super::{FromVal, ToVal, shape};
use wasm_junction_core::{CallError, Val};
use wasmtime_wasi_http::p2::bindings::http::types::{
    DnsErrorPayload as P2DnsErrorPayload, ErrorCode as P2ErrorCode,
    FieldSizePayload as P2FieldSizePayload, Method as P2Method, Scheme as P2Scheme,
    TlsAlertReceivedPayload as P2TlsAlertReceivedPayload,
};

#[cfg(feature = "wasi-p3")]
use wasmtime_wasi_http::p3::bindings::http::types::{
    DnsErrorPayload as P3DnsErrorPayload, ErrorCode as P3ErrorCode,
    FieldSizePayload as P3FieldSizePayload, HeaderError as P3HeaderError, Method as P3Method,
    RequestOptionsError as P3RequestOptionsError, Scheme as P3Scheme,
    TlsAlertReceivedPayload as P3TlsAlertReceivedPayload,
};

fn variant(case: &str, value: Option<Val>) -> Val {
    Val::Variant {
        case: case.to_owned(),
        value: value.map(Box::new),
    }
}

fn record(fields: impl IntoIterator<Item = (&'static str, Val)>) -> Val {
    Val::Record(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn payload<T: FromVal>(value: Option<Box<Val>>, expected: &str) -> Result<T, CallError> {
    value
        .ok_or_else(|| shape(expected))
        .and_then(|value| T::from_val(*value))
}

fn fields<const N: usize>(value: Val, expected: &str) -> Result<[Val; N], CallError> {
    let Val::Record(fields) = value else {
        return Err(shape(expected));
    };
    fields
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| shape(expected))
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the surrounding decoder moves payloads in its other match arms"
)]
fn without_payload<T>(value: Option<Box<Val>>, result: T) -> Result<T, CallError> {
    value
        .is_none()
        .then_some(result)
        .ok_or_else(|| shape("payload-free variant"))
}

macro_rules! error_enum {
    ($ty:ty, $expected:literal, $other:ty, $($case:literal => $variant:ident),+ $(,)?) => {
        impl ToVal for $ty {
            fn to_val(self) -> Val {
                match self {
                    $(Self::$variant => variant($case, None),)+
                    Self::Other(value) => variant("other", Some(value.to_val())),
                }
            }
        }

        impl FromVal for $ty {
            fn from_val(value: Val) -> Result<Self, CallError> {
                let Val::Variant { case, value } = value else {
                    return Err(shape($expected));
                };
                match (case.as_str(), value) {
                    $(($case, None) => Ok(Self::$variant),)+
                    ("other", Some(value)) => <$other>::from_val(*value).map(Self::Other),
                    _ => Err(shape(concat!($expected, " case"))),
                }
            }
        }
    };
}

macro_rules! error_code {
    ($error:ty, $dns:ident, $field_size:ident, $tls:ident) => {
        impl ToVal for $error {
            #[allow(clippy::too_many_lines, reason = "the WIT variant has 39 cases")]
            fn to_val(self) -> Val {
                let (case, value) = match self {
                    Self::DnsTimeout => ("DNS-timeout", None),
                    Self::DnsError(value) => (
                        "DNS-error",
                        Some(record([
                            ("rcode", value.rcode.to_val()),
                            ("info-code", value.info_code.to_val()),
                        ])),
                    ),
                    Self::DestinationNotFound => ("destination-not-found", None),
                    Self::DestinationUnavailable => ("destination-unavailable", None),
                    Self::DestinationIpProhibited => ("destination-IP-prohibited", None),
                    Self::DestinationIpUnroutable => ("destination-IP-unroutable", None),
                    Self::ConnectionRefused => ("connection-refused", None),
                    Self::ConnectionTerminated => ("connection-terminated", None),
                    Self::ConnectionTimeout => ("connection-timeout", None),
                    Self::ConnectionReadTimeout => ("connection-read-timeout", None),
                    Self::ConnectionWriteTimeout => ("connection-write-timeout", None),
                    Self::ConnectionLimitReached => ("connection-limit-reached", None),
                    Self::TlsProtocolError => ("TLS-protocol-error", None),
                    Self::TlsCertificateError => ("TLS-certificate-error", None),
                    Self::TlsAlertReceived(value) => (
                        "TLS-alert-received",
                        Some(record([
                            ("alert-id", value.alert_id.to_val()),
                            ("alert-message", value.alert_message.to_val()),
                        ])),
                    ),
                    Self::HttpRequestDenied => ("HTTP-request-denied", None),
                    Self::HttpRequestLengthRequired => ("HTTP-request-length-required", None),
                    Self::HttpRequestBodySize(value) => {
                        ("HTTP-request-body-size", Some(value.to_val()))
                    }
                    Self::HttpRequestMethodInvalid => ("HTTP-request-method-invalid", None),
                    Self::HttpRequestUriInvalid => ("HTTP-request-URI-invalid", None),
                    Self::HttpRequestUriTooLong => ("HTTP-request-URI-too-long", None),
                    Self::HttpRequestHeaderSectionSize(value) => {
                        ("HTTP-request-header-section-size", Some(value.to_val()))
                    }
                    Self::HttpRequestHeaderSize(value) => (
                        "HTTP-request-header-size",
                        Some(Val::Option(value.map(|value| {
                            Box::new(record([
                                ("field-name", value.field_name.to_val()),
                                ("field-size", value.field_size.to_val()),
                            ]))
                        }))),
                    ),
                    Self::HttpRequestTrailerSectionSize(value) => {
                        ("HTTP-request-trailer-section-size", Some(value.to_val()))
                    }
                    Self::HttpRequestTrailerSize(value) => (
                        "HTTP-request-trailer-size",
                        Some(record([
                            ("field-name", value.field_name.to_val()),
                            ("field-size", value.field_size.to_val()),
                        ])),
                    ),
                    Self::HttpResponseIncomplete => ("HTTP-response-incomplete", None),
                    Self::HttpResponseHeaderSectionSize(value) => {
                        ("HTTP-response-header-section-size", Some(value.to_val()))
                    }
                    Self::HttpResponseHeaderSize(value) => (
                        "HTTP-response-header-size",
                        Some(record([
                            ("field-name", value.field_name.to_val()),
                            ("field-size", value.field_size.to_val()),
                        ])),
                    ),
                    Self::HttpResponseBodySize(value) => {
                        ("HTTP-response-body-size", Some(value.to_val()))
                    }
                    Self::HttpResponseTrailerSectionSize(value) => {
                        ("HTTP-response-trailer-section-size", Some(value.to_val()))
                    }
                    Self::HttpResponseTrailerSize(value) => (
                        "HTTP-response-trailer-size",
                        Some(record([
                            ("field-name", value.field_name.to_val()),
                            ("field-size", value.field_size.to_val()),
                        ])),
                    ),
                    Self::HttpResponseTransferCoding(value) => {
                        ("HTTP-response-transfer-coding", Some(value.to_val()))
                    }
                    Self::HttpResponseContentCoding(value) => {
                        ("HTTP-response-content-coding", Some(value.to_val()))
                    }
                    Self::HttpResponseTimeout => ("HTTP-response-timeout", None),
                    Self::HttpUpgradeFailed => ("HTTP-upgrade-failed", None),
                    Self::HttpProtocolError => ("HTTP-protocol-error", None),
                    Self::LoopDetected => ("loop-detected", None),
                    Self::ConfigurationError => ("configuration-error", None),
                    Self::InternalError(value) => ("internal-error", Some(value.to_val())),
                };
                variant(case, value)
            }
        }

        impl FromVal for $error {
            #[allow(clippy::too_many_lines, reason = "the WIT variant has 39 cases")]
            fn from_val(value: Val) -> Result<Self, CallError> {
                let Val::Variant { case, value } = value else {
                    return Err(shape("error-code"));
                };
                match case.as_str() {
                    "DNS-timeout" => without_payload(value, Self::DnsTimeout),
                    "DNS-error" => {
                        let [rcode, info] =
                            fields(*value.ok_or_else(|| shape("DNS-error"))?, "DNS-error")?;
                        Ok(Self::DnsError($dns {
                            rcode: Option::from_val(rcode)?,
                            info_code: Option::from_val(info)?,
                        }))
                    }
                    "destination-not-found" => without_payload(value, Self::DestinationNotFound),
                    "destination-unavailable" => {
                        without_payload(value, Self::DestinationUnavailable)
                    }
                    "destination-IP-prohibited" => {
                        without_payload(value, Self::DestinationIpProhibited)
                    }
                    "destination-IP-unroutable" => {
                        without_payload(value, Self::DestinationIpUnroutable)
                    }
                    "connection-refused" => without_payload(value, Self::ConnectionRefused),
                    "connection-terminated" => without_payload(value, Self::ConnectionTerminated),
                    "connection-timeout" => without_payload(value, Self::ConnectionTimeout),
                    "connection-read-timeout" => {
                        without_payload(value, Self::ConnectionReadTimeout)
                    }
                    "connection-write-timeout" => {
                        without_payload(value, Self::ConnectionWriteTimeout)
                    }
                    "connection-limit-reached" => {
                        without_payload(value, Self::ConnectionLimitReached)
                    }
                    "TLS-protocol-error" => without_payload(value, Self::TlsProtocolError),
                    "TLS-certificate-error" => without_payload(value, Self::TlsCertificateError),
                    "TLS-alert-received" => {
                        let [id, message] =
                            fields(*value.ok_or_else(|| shape("TLS alert"))?, "TLS alert")?;
                        Ok(Self::TlsAlertReceived($tls {
                            alert_id: Option::from_val(id)?,
                            alert_message: Option::from_val(message)?,
                        }))
                    }
                    "HTTP-request-denied" => without_payload(value, Self::HttpRequestDenied),
                    "HTTP-request-length-required" => {
                        without_payload(value, Self::HttpRequestLengthRequired)
                    }
                    "HTTP-request-body-size" => {
                        payload(value, "body size").map(Self::HttpRequestBodySize)
                    }
                    "HTTP-request-method-invalid" => {
                        without_payload(value, Self::HttpRequestMethodInvalid)
                    }
                    "HTTP-request-URI-invalid" => {
                        without_payload(value, Self::HttpRequestUriInvalid)
                    }
                    "HTTP-request-URI-too-long" => {
                        without_payload(value, Self::HttpRequestUriTooLong)
                    }
                    "HTTP-request-header-section-size" => payload(value, "header section size")
                        .map(Self::HttpRequestHeaderSectionSize),
                    "HTTP-request-header-size" => {
                        let Val::Option(value) = *value.ok_or_else(|| shape("header size"))? else {
                            return Err(shape("optional header size"));
                        };
                        Ok(Self::HttpRequestHeaderSize(
                            value
                                .map(|value| -> Result<$field_size, CallError> {
                                    let [name, size] = fields(*value, "field-size-payload")?;
                                    Ok($field_size {
                                        field_name: Option::from_val(name)?,
                                        field_size: Option::from_val(size)?,
                                    })
                                })
                                .transpose()?,
                        ))
                    }
                    "HTTP-request-trailer-section-size" => payload(value, "trailer section size")
                        .map(Self::HttpRequestTrailerSectionSize),
                    "HTTP-request-trailer-size" => {
                        let [name, size] = fields(
                            *value.ok_or_else(|| shape("trailer size"))?,
                            "field-size-payload",
                        )?;
                        Ok(Self::HttpRequestTrailerSize($field_size {
                            field_name: Option::from_val(name)?,
                            field_size: Option::from_val(size)?,
                        }))
                    }
                    "HTTP-response-incomplete" => {
                        without_payload(value, Self::HttpResponseIncomplete)
                    }
                    "HTTP-response-header-section-size" => payload(value, "header section size")
                        .map(Self::HttpResponseHeaderSectionSize),
                    "HTTP-response-header-size" => {
                        let [name, size] = fields(
                            *value.ok_or_else(|| shape("header size"))?,
                            "field-size-payload",
                        )?;
                        Ok(Self::HttpResponseHeaderSize($field_size {
                            field_name: Option::from_val(name)?,
                            field_size: Option::from_val(size)?,
                        }))
                    }
                    "HTTP-response-body-size" => {
                        payload(value, "body size").map(Self::HttpResponseBodySize)
                    }
                    "HTTP-response-trailer-section-size" => payload(value, "trailer section size")
                        .map(Self::HttpResponseTrailerSectionSize),
                    "HTTP-response-trailer-size" => {
                        let [name, size] = fields(
                            *value.ok_or_else(|| shape("trailer size"))?,
                            "field-size-payload",
                        )?;
                        Ok(Self::HttpResponseTrailerSize($field_size {
                            field_name: Option::from_val(name)?,
                            field_size: Option::from_val(size)?,
                        }))
                    }
                    "HTTP-response-transfer-coding" => {
                        payload(value, "transfer coding").map(Self::HttpResponseTransferCoding)
                    }
                    "HTTP-response-content-coding" => {
                        payload(value, "content coding").map(Self::HttpResponseContentCoding)
                    }
                    "HTTP-response-timeout" => without_payload(value, Self::HttpResponseTimeout),
                    "HTTP-upgrade-failed" => without_payload(value, Self::HttpUpgradeFailed),
                    "HTTP-protocol-error" => without_payload(value, Self::HttpProtocolError),
                    "loop-detected" => without_payload(value, Self::LoopDetected),
                    "configuration-error" => without_payload(value, Self::ConfigurationError),
                    "internal-error" => payload(value, "internal error").map(Self::InternalError),
                    _ => Err(shape("error-code case")),
                }
            }
        }
    };
}

macro_rules! method_codec {
    ($ty:ty) => {
        error_enum!($ty, "method", String,
            "get" => Get, "head" => Head, "post" => Post, "put" => Put,
            "delete" => Delete, "connect" => Connect, "options" => Options,
            "trace" => Trace, "patch" => Patch);
    };
}

macro_rules! scheme_codec {
    ($ty:ty) => {
        error_enum!($ty, "scheme", String, "HTTP" => Http, "HTTPS" => Https);
    };
}

method_codec!(P2Method);
scheme_codec!(P2Scheme);
error_code!(
    P2ErrorCode,
    P2DnsErrorPayload,
    P2FieldSizePayload,
    P2TlsAlertReceivedPayload
);

#[cfg(feature = "wasi-p3")]
method_codec!(P3Method);
#[cfg(feature = "wasi-p3")]
scheme_codec!(P3Scheme);
#[cfg(feature = "wasi-p3")]
error_enum!(P3HeaderError, "header-error", Option<String>,
    "invalid-syntax" => InvalidSyntax, "forbidden" => Forbidden,
    "immutable" => Immutable, "size-exceeded" => SizeExceeded);
#[cfg(feature = "wasi-p3")]
error_enum!(P3RequestOptionsError, "request-options-error", Option<String>,
    "not-supported" => NotSupported, "immutable" => Immutable);
#[cfg(feature = "wasi-p3")]
error_code!(
    P3ErrorCode,
    P3DnsErrorPayload,
    P3FieldSizePayload,
    P3TlsAlertReceivedPayload
);
