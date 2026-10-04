use super::super::ToVal;
use super::codec::ToHttpVal;
use wasm_junction_core::Val;
use wasmtime_wasi_http::p2::bindings::http::types::{ErrorCode, FieldSizePayload};

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
