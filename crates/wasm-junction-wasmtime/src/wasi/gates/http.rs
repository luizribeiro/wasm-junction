use super::{FromVal, ToVal, WitResource, shape};
use wasm_junction_core::{CallError, Val};
use wasmtime_wasi_http::p3::bindings::http::types::{
    Fields, HeaderError, Method, Request, RequestOptions, RequestOptionsError, Response, Scheme,
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
}
