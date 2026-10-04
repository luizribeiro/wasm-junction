use super::super::{FromVal, ToVal, shape};
use wasm_junction_core::{CallError, Val};
use wasmtime_wasi_http::p2::bindings::http::types::{HeaderError, Method, Scheme};

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
        match (case.as_str(), value) {
            ("get", None) => Ok(Self::Get),
            ("head", None) => Ok(Self::Head),
            ("post", None) => Ok(Self::Post),
            ("put", None) => Ok(Self::Put),
            ("delete", None) => Ok(Self::Delete),
            ("connect", None) => Ok(Self::Connect),
            ("options", None) => Ok(Self::Options),
            ("trace", None) => Ok(Self::Trace),
            ("patch", None) => Ok(Self::Patch),
            ("other", Some(value)) => String::from_val(*value).map(Self::Other),
            _ => Err(shape("method case")),
        }
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

impl ToVal for HeaderError {
    fn to_val(self) -> Val {
        variant(
            match self {
                Self::InvalidSyntax => "invalid-syntax",
                Self::Forbidden => "forbidden",
                Self::Immutable => "immutable",
            },
            None,
        )
    }
}

impl FromVal for HeaderError {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Variant { case, value: None } = value else {
            return Err(shape("header-error"));
        };
        match case.as_str() {
            "invalid-syntax" => Ok(Self::InvalidSyntax),
            "forbidden" => Ok(Self::Forbidden),
            "immutable" => Ok(Self::Immutable),
            _ => Err(shape("header-error case")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_values_round_trip() {
        assert!(matches!(
            Method::from_val(Method::Post.to_val()).unwrap(),
            Method::Post
        ));
        assert!(matches!(
            Scheme::from_val(Scheme::Https.to_val()).unwrap(),
            Scheme::Https
        ));
        assert!(matches!(
            HeaderError::from_val(HeaderError::Immutable.to_val()).unwrap(),
            HeaderError::Immutable
        ));
    }
}
