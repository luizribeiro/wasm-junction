use super::super::{FromVal, ToVal, shape};
use super::codec::{FromHttpVal, ToHttpVal};
use wasm_junction_core::{CallError, Val};
use wasmtime_wasi_http::p2::bindings::http::types::{
    DnsErrorPayload, ErrorCode, FieldSizePayload, TlsAlertReceivedPayload,
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

fn field_size(value: FieldSizePayload) -> Val {
    record([
        ("field-name", value.field_name.to_val()),
        ("field-size", value.field_size.to_val()),
    ])
}

impl ToVal for ErrorCode {
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
            Self::HttpRequestBodySize(value) => ("HTTP-request-body-size", Some(value.to_val())),
            Self::HttpRequestMethodInvalid => ("HTTP-request-method-invalid", None),
            Self::HttpRequestUriInvalid => ("HTTP-request-URI-invalid", None),
            Self::HttpRequestUriTooLong => ("HTTP-request-URI-too-long", None),
            Self::HttpRequestHeaderSectionSize(value) => {
                ("HTTP-request-header-section-size", Some(value.to_val()))
            }
            Self::HttpRequestHeaderSize(value) => (
                "HTTP-request-header-size",
                Some(Val::Option(value.map(|value| Box::new(field_size(value))))),
            ),
            Self::HttpRequestTrailerSectionSize(value) => {
                ("HTTP-request-trailer-section-size", Some(value.to_val()))
            }
            Self::HttpRequestTrailerSize(value) => {
                ("HTTP-request-trailer-size", Some(field_size(value)))
            }
            Self::HttpResponseIncomplete => ("HTTP-response-incomplete", None),
            Self::HttpResponseHeaderSectionSize(value) => {
                ("HTTP-response-header-section-size", Some(value.to_val()))
            }
            Self::HttpResponseHeaderSize(value) => {
                ("HTTP-response-header-size", Some(field_size(value)))
            }
            Self::HttpResponseBodySize(value) => ("HTTP-response-body-size", Some(value.to_val())),
            Self::HttpResponseTrailerSectionSize(value) => {
                ("HTTP-response-trailer-section-size", Some(value.to_val()))
            }
            Self::HttpResponseTrailerSize(value) => {
                ("HTTP-response-trailer-size", Some(field_size(value)))
            }
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

impl ToHttpVal for ErrorCode {
    fn to_http_val(self) -> Val {
        self.to_val()
    }
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

fn field_size_from_val(value: Val) -> Result<FieldSizePayload, CallError> {
    let [name, size] = fields(value, "field-size-payload")?;
    Ok(FieldSizePayload {
        field_name: Option::<String>::from_val(name)?,
        field_size: Option::<u32>::from_val(size)?,
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the surrounding decoder moves payloads in its other match arms"
)]
fn unit(value: Option<Box<Val>>, code: ErrorCode) -> Result<ErrorCode, CallError> {
    value
        .is_none()
        .then_some(code)
        .ok_or_else(|| shape("payload-free error-code"))
}

impl FromVal for ErrorCode {
    #[allow(clippy::too_many_lines, reason = "the WIT variant has 39 cases")]
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Variant { case, value } = value else {
            return Err(shape("error-code"));
        };
        match case.as_str() {
            "DNS-timeout" => unit(value, Self::DnsTimeout),
            "DNS-error" => {
                let [rcode, info] = fields(*value.ok_or_else(|| shape("DNS-error"))?, "DNS-error")?;
                Ok(Self::DnsError(DnsErrorPayload {
                    rcode: Option::from_val(rcode)?,
                    info_code: Option::from_val(info)?,
                }))
            }
            "destination-not-found" => unit(value, Self::DestinationNotFound),
            "destination-unavailable" => unit(value, Self::DestinationUnavailable),
            "destination-IP-prohibited" => unit(value, Self::DestinationIpProhibited),
            "destination-IP-unroutable" => unit(value, Self::DestinationIpUnroutable),
            "connection-refused" => unit(value, Self::ConnectionRefused),
            "connection-terminated" => unit(value, Self::ConnectionTerminated),
            "connection-timeout" => unit(value, Self::ConnectionTimeout),
            "connection-read-timeout" => unit(value, Self::ConnectionReadTimeout),
            "connection-write-timeout" => unit(value, Self::ConnectionWriteTimeout),
            "connection-limit-reached" => unit(value, Self::ConnectionLimitReached),
            "TLS-protocol-error" => unit(value, Self::TlsProtocolError),
            "TLS-certificate-error" => unit(value, Self::TlsCertificateError),
            "TLS-alert-received" => {
                let [id, message] = fields(*value.ok_or_else(|| shape("TLS alert"))?, "TLS alert")?;
                Ok(Self::TlsAlertReceived(TlsAlertReceivedPayload {
                    alert_id: Option::from_val(id)?,
                    alert_message: Option::from_val(message)?,
                }))
            }
            "HTTP-request-denied" => unit(value, Self::HttpRequestDenied),
            "HTTP-request-length-required" => unit(value, Self::HttpRequestLengthRequired),
            "HTTP-request-body-size" => payload(value, "body size").map(Self::HttpRequestBodySize),
            "HTTP-request-method-invalid" => unit(value, Self::HttpRequestMethodInvalid),
            "HTTP-request-URI-invalid" => unit(value, Self::HttpRequestUriInvalid),
            "HTTP-request-URI-too-long" => unit(value, Self::HttpRequestUriTooLong),
            "HTTP-request-header-section-size" => {
                payload(value, "header section size").map(Self::HttpRequestHeaderSectionSize)
            }
            "HTTP-request-header-size" => {
                let Val::Option(value) = *value.ok_or_else(|| shape("header size"))? else {
                    return Err(shape("header size"));
                };
                Ok(Self::HttpRequestHeaderSize(
                    value.map(|value| field_size_from_val(*value)).transpose()?,
                ))
            }
            "HTTP-request-trailer-section-size" => {
                payload(value, "trailer section size").map(Self::HttpRequestTrailerSectionSize)
            }
            "HTTP-request-trailer-size" => value
                .ok_or_else(|| shape("trailer size"))
                .and_then(|value| field_size_from_val(*value))
                .map(Self::HttpRequestTrailerSize),
            "HTTP-response-incomplete" => unit(value, Self::HttpResponseIncomplete),
            "HTTP-response-header-section-size" => {
                payload(value, "header section size").map(Self::HttpResponseHeaderSectionSize)
            }
            "HTTP-response-header-size" => value
                .ok_or_else(|| shape("header size"))
                .and_then(|value| field_size_from_val(*value))
                .map(Self::HttpResponseHeaderSize),
            "HTTP-response-body-size" => {
                payload(value, "body size").map(Self::HttpResponseBodySize)
            }
            "HTTP-response-trailer-section-size" => {
                payload(value, "trailer section size").map(Self::HttpResponseTrailerSectionSize)
            }
            "HTTP-response-trailer-size" => value
                .ok_or_else(|| shape("trailer size"))
                .and_then(|value| field_size_from_val(*value))
                .map(Self::HttpResponseTrailerSize),
            "HTTP-response-transfer-coding" => {
                payload(value, "transfer coding").map(Self::HttpResponseTransferCoding)
            }
            "HTTP-response-content-coding" => {
                payload(value, "content coding").map(Self::HttpResponseContentCoding)
            }
            "HTTP-response-timeout" => unit(value, Self::HttpResponseTimeout),
            "HTTP-upgrade-failed" => unit(value, Self::HttpUpgradeFailed),
            "HTTP-protocol-error" => unit(value, Self::HttpProtocolError),
            "loop-detected" => unit(value, Self::LoopDetected),
            "configuration-error" => unit(value, Self::ConfigurationError),
            "internal-error" => payload(value, "internal error").map(Self::InternalError),
            _ => Err(shape("error-code case")),
        }
    }
}

impl FromHttpVal for ErrorCode {
    fn from_http_val(value: Val) -> Result<Self, CallError> {
        Self::from_val(value)
    }
}
