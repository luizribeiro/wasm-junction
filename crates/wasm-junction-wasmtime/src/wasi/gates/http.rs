use super::{FromVal, ToVal, WitResource, shape};
use wasm_junction_core::{CallError, Val};
use wasmtime_wasi_http::p3::bindings::http::types::{
    DnsErrorPayload, ErrorCode, FieldSizePayload, Fields, HeaderError, Method, Request,
    RequestOptions, RequestOptionsError, Response, Scheme, TlsAlertReceivedPayload,
};

const TYPES: &str = "wasi:http/types@0.3.0";

macro_rules! resource {
    ($ty:ty, $name:literal) => {
        impl WitResource for $ty {
            const INTERFACE: &'static str = TYPES;
            const NAME: &'static str = $name;
        }
    };
}

resource!(Fields, "fields");
resource!(Request, "request");
resource!(RequestOptions, "request-options");
resource!(Response, "response");

fn variant(case: &str, value: Option<Val>) -> Val {
    Val::Variant {
        case: case.to_owned(),
        value: value.map(Box::new),
    }
}

impl ToVal for Method {
    fn to_val(self) -> Val {
        let (case, value) = match self {
            Self::Get => ("get", None),
            Self::Head => ("head", None),
            Self::Post => ("post", None),
            Self::Put => ("put", None),
            Self::Delete => ("delete", None),
            Self::Connect => ("connect", None),
            Self::Options => ("options", None),
            Self::Trace => ("trace", None),
            Self::Patch => ("patch", None),
            Self::Other(value) => ("other", Some(value.to_val())),
        };
        variant(case, value)
    }
}

impl FromVal for Method {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Variant { case, value } = value else {
            return Err(shape("method"));
        };
        Ok(match (case.as_str(), value) {
            ("get", None) => Self::Get,
            ("head", None) => Self::Head,
            ("post", None) => Self::Post,
            ("put", None) => Self::Put,
            ("delete", None) => Self::Delete,
            ("connect", None) => Self::Connect,
            ("options", None) => Self::Options,
            ("trace", None) => Self::Trace,
            ("patch", None) => Self::Patch,
            ("other", Some(value)) => Self::Other(String::from_val(*value)?),
            _ => return Err(shape("method case")),
        })
    }
}

impl ToVal for Scheme {
    fn to_val(self) -> Val {
        match self {
            Self::Http => variant("HTTP", None),
            Self::Https => variant("HTTPS", None),
            Self::Other(value) => variant("other", Some(value.to_val())),
        }
    }
}

impl FromVal for Scheme {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Variant { case, value } = value else {
            return Err(shape("scheme"));
        };
        match (case.as_str(), value) {
            ("HTTP", None) => Ok(Self::Http),
            ("HTTPS", None) => Ok(Self::Https),
            ("other", Some(value)) => String::from_val(*value).map(Self::Other),
            _ => Err(shape("scheme case")),
        }
    }
}

macro_rules! error_enum {
    ($ty:ty, $expected:literal, $($case:literal => $variant:ident),+ $(,)?) => {
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
                let Val::Variant { case, value } = value else { return Err(shape($expected)); };
                match (case.as_str(), value) {
                    $(($case, None) => Ok(Self::$variant),)+
                    ("other", Some(value)) => Option::<String>::from_val(*value).map(Self::Other),
                    _ => Err(shape(concat!($expected, " case"))),
                }
            }
        }
    };
}

error_enum!(HeaderError, "header-error",
    "invalid-syntax" => InvalidSyntax, "forbidden" => Forbidden,
    "immutable" => Immutable, "size-exceeded" => SizeExceeded);
error_enum!(RequestOptionsError, "request-options-error",
    "not-supported" => NotSupported, "immutable" => Immutable);

fn record(fields: impl IntoIterator<Item = (&'static str, Val)>) -> Val {
    Val::Record(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn field_size_value(payload: FieldSizePayload) -> Val {
    record([
        ("field-name", payload.field_name.to_val()),
        ("field-size", payload.field_size.to_val()),
    ])
}

impl ToVal for ErrorCode {
    #[allow(clippy::too_many_lines, reason = "the WIT variant has 38 cases")]
    fn to_val(self) -> Val {
        let (case, value) = match self {
            Self::DnsTimeout => ("DNS-timeout", None),
            Self::DnsError(payload) => (
                "DNS-error",
                Some(record([
                    ("rcode", payload.rcode.to_val()),
                    ("info-code", payload.info_code.to_val()),
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
            Self::TlsAlertReceived(payload) => (
                "TLS-alert-received",
                Some(record([
                    ("alert-id", payload.alert_id.to_val()),
                    ("alert-message", payload.alert_message.to_val()),
                ])),
            ),
            Self::HttpRequestDenied => ("HTTP-request-denied", None),
            Self::HttpRequestLengthRequired => ("HTTP-request-length-required", None),
            Self::HttpRequestBodySize(payload) => {
                ("HTTP-request-body-size", Some(payload.to_val()))
            }
            Self::HttpRequestMethodInvalid => ("HTTP-request-method-invalid", None),
            Self::HttpRequestUriInvalid => ("HTTP-request-URI-invalid", None),
            Self::HttpRequestUriTooLong => ("HTTP-request-URI-too-long", None),
            Self::HttpRequestHeaderSectionSize(payload) => {
                ("HTTP-request-header-section-size", Some(payload.to_val()))
            }
            Self::HttpRequestHeaderSize(payload) => (
                "HTTP-request-header-size",
                Some(Val::Option(
                    payload.map(|payload| Box::new(field_size_value(payload))),
                )),
            ),
            Self::HttpRequestTrailerSectionSize(payload) => {
                ("HTTP-request-trailer-section-size", Some(payload.to_val()))
            }
            Self::HttpRequestTrailerSize(payload) => {
                ("HTTP-request-trailer-size", Some(field_size_value(payload)))
            }
            Self::HttpResponseIncomplete => ("HTTP-response-incomplete", None),
            Self::HttpResponseHeaderSectionSize(payload) => {
                ("HTTP-response-header-section-size", Some(payload.to_val()))
            }
            Self::HttpResponseHeaderSize(payload) => {
                ("HTTP-response-header-size", Some(field_size_value(payload)))
            }
            Self::HttpResponseBodySize(payload) => {
                ("HTTP-response-body-size", Some(payload.to_val()))
            }
            Self::HttpResponseTrailerSectionSize(payload) => {
                ("HTTP-response-trailer-section-size", Some(payload.to_val()))
            }
            Self::HttpResponseTrailerSize(payload) => (
                "HTTP-response-trailer-size",
                Some(field_size_value(payload)),
            ),
            Self::HttpResponseTransferCoding(payload) => {
                ("HTTP-response-transfer-coding", Some(payload.to_val()))
            }
            Self::HttpResponseContentCoding(payload) => {
                ("HTTP-response-content-coding", Some(payload.to_val()))
            }
            Self::HttpResponseTimeout => ("HTTP-response-timeout", None),
            Self::HttpUpgradeFailed => ("HTTP-upgrade-failed", None),
            Self::HttpProtocolError => ("HTTP-protocol-error", None),
            Self::LoopDetected => ("loop-detected", None),
            Self::ConfigurationError => ("configuration-error", None),
            Self::InternalError(payload) => ("internal-error", Some(payload.to_val())),
        };
        variant(case, value)
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

fn no_payload(value: &Option<Box<Val>>) -> Result<(), CallError> {
    if value.is_none() {
        Ok(())
    } else {
        Err(shape("payload-free error-code"))
    }
}

fn decode_transport_error(
    case: &str,
    value: Option<Box<Val>>,
) -> Option<Result<ErrorCode, CallError>> {
    let decoded = match case {
        "DNS-timeout" => no_payload(&value).map(|()| ErrorCode::DnsTimeout),
        "DNS-error" => (|| {
            let [rcode, info_code] = fields(
                *value.ok_or_else(|| shape("DNS-error payload"))?,
                "DNS-error payload",
            )?;
            Ok(ErrorCode::DnsError(DnsErrorPayload {
                rcode: Option::<String>::from_val(rcode)?,
                info_code: Option::<u16>::from_val(info_code)?,
            }))
        })(),
        "destination-not-found" => no_payload(&value).map(|()| ErrorCode::DestinationNotFound),
        "destination-unavailable" => no_payload(&value).map(|()| ErrorCode::DestinationUnavailable),
        "destination-IP-prohibited" => {
            no_payload(&value).map(|()| ErrorCode::DestinationIpProhibited)
        }
        "destination-IP-unroutable" => {
            no_payload(&value).map(|()| ErrorCode::DestinationIpUnroutable)
        }
        "connection-refused" => no_payload(&value).map(|()| ErrorCode::ConnectionRefused),
        "connection-terminated" => no_payload(&value).map(|()| ErrorCode::ConnectionTerminated),
        "connection-timeout" => no_payload(&value).map(|()| ErrorCode::ConnectionTimeout),
        "connection-read-timeout" => no_payload(&value).map(|()| ErrorCode::ConnectionReadTimeout),
        "connection-write-timeout" => {
            no_payload(&value).map(|()| ErrorCode::ConnectionWriteTimeout)
        }
        "connection-limit-reached" => {
            no_payload(&value).map(|()| ErrorCode::ConnectionLimitReached)
        }
        "TLS-protocol-error" => no_payload(&value).map(|()| ErrorCode::TlsProtocolError),
        "TLS-certificate-error" => no_payload(&value).map(|()| ErrorCode::TlsCertificateError),
        "TLS-alert-received" => (|| {
            let [alert_id, alert_message] = fields(
                *value.ok_or_else(|| shape("TLS alert payload"))?,
                "TLS alert payload",
            )?;
            Ok(ErrorCode::TlsAlertReceived(TlsAlertReceivedPayload {
                alert_id: Option::<u8>::from_val(alert_id)?,
                alert_message: Option::<String>::from_val(alert_message)?,
            }))
        })(),
        _ => return None,
    };
    Some(decoded)
}

impl FromVal for ErrorCode {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Variant { case, value } = value else {
            return Err(shape("error-code"));
        };
        if let Some(decoded) = decode_transport_error(&case, value.clone()) {
            return decoded;
        }
        match case.as_str() {
            "HTTP-request-denied" => no_payload(&value).map(|()| Self::HttpRequestDenied),
            "internal-error" => payload(value, "internal error").map(Self::InternalError),
            _ => Err(shape("error-code case")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_variants_round_trip() {
        assert!(matches!(
            Method::from_val(Method::Patch.to_val()),
            Ok(Method::Patch)
        ));
        assert!(matches!(
            Scheme::from_val(Scheme::Https.to_val()),
            Ok(Scheme::Https)
        ));
        assert!(matches!(
            HeaderError::from_val(HeaderError::Forbidden.to_val()),
            Ok(HeaderError::Forbidden)
        ));
    }

    #[test]
    fn http_error_codes_have_wit_case_names() {
        assert_eq!(
            ErrorCode::HttpRequestDenied.to_val(),
            variant("HTTP-request-denied", None)
        );
        assert_eq!(
            ErrorCode::HttpRequestHeaderSize(Some(FieldSizePayload {
                field_name: Some("authorization".to_owned()),
                field_size: Some(99),
            }))
            .to_val(),
            variant(
                "HTTP-request-header-size",
                Some(Val::Option(Some(Box::new(field_size_value(
                    FieldSizePayload {
                        field_name: Some("authorization".to_owned()),
                        field_size: Some(99),
                    }
                ))))),
            )
        );
        let error = ErrorCode::DnsError(DnsErrorPayload {
            rcode: Some("refused".to_owned()),
            info_code: Some(5),
        });
        let ErrorCode::DnsError(decoded) = ErrorCode::from_val(error.to_val()).unwrap() else {
            panic!("DNS error changed case");
        };
        assert_eq!(decoded.rcode.as_deref(), Some("refused"));
        assert_eq!(decoded.info_code, Some(5));
    }
}
