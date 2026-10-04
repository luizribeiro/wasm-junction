use super::super::{FromVal, ToVal, shape};
use wasm_junction_core::{CallError, Val};
use wasmtime_wasi_http::p2::bindings::http::types::HeaderError;

fn variant(case: &str) -> Val {
    Val::Variant {
        case: case.to_owned(),
        value: None,
    }
}

impl ToVal for HeaderError {
    fn to_val(self) -> Val {
        variant(match self {
            Self::InvalidSyntax => "invalid-syntax",
            Self::Forbidden => "forbidden",
            Self::Immutable => "immutable",
        })
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
    fn header_error_round_trips() {
        assert!(matches!(
            HeaderError::from_val(HeaderError::Immutable.to_val()),
            Ok(HeaderError::Immutable)
        ));
    }
}
