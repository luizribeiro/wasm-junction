use std::error::Error;
use std::fmt::{self, Display};

use crate::{Resource, StreamHandle};

/// An engine-neutral representation of a plain WIT value.
///
/// Generated bindings convert their Rust types to and from this representation at call
/// boundaries. New framework value kinds may be added without a breaking change.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Val {
    /// A WIT `bool`.
    Bool(bool),
    /// A WIT `s8`.
    S8(i8),
    /// A WIT `u8`.
    U8(u8),
    /// A WIT `s16`.
    S16(i16),
    /// A WIT `u16`.
    U16(u16),
    /// A WIT `s32`.
    S32(i32),
    /// A WIT `u32`.
    U32(u32),
    /// A WIT `s64`.
    S64(i64),
    /// A WIT `u64`.
    U64(u64),
    /// A WIT `f32`.
    F32(f32),
    /// A WIT `f64`.
    F64(f64),
    /// A WIT `char`.
    Char(char),
    /// A WIT `string`.
    String(String),
    /// A WIT `list<u8>`.
    Bytes(Vec<u8>),
    /// A WIT `list`, in element order.
    List(Vec<Self>),
    /// A WIT `tuple`, in element order.
    Tuple(Vec<Self>),
    /// A WIT `record`, in field declaration order.
    Record(Vec<(String, Self)>),
    /// A WIT `variant` case with its optional payload.
    Variant {
        /// The case name from the WIT definition.
        case: String,
        /// The case payload, or `None` for a payload-free case.
        value: Option<Box<Self>>,
    },
    /// A WIT `enum` case name.
    Enum(String),
    /// The active names in a WIT `flags` value.
    Flags(Vec<String>),
    /// A WIT `option` payload.
    Option(Option<Box<Self>>),
    /// A WIT `result`, whose success and error payloads may each be absent.
    Result(Result<Option<Box<Self>>, Option<Box<Self>>>),
    /// A host-defined WIT resource handle.
    Resource(Resource),
    /// A WIT `stream<u8>` handle.
    Stream(StreamHandle),
}

/// A sequence of engine-neutral WIT values used for call arguments and results.
pub type Vals = Vec<Val>;

macro_rules! primitive_conversion {
    ($type:ty, $variant:ident, $wit:literal) => {
        impl From<$type> for Val {
            fn from(value: $type) -> Self {
                Self::$variant(value)
            }
        }

        impl TryFrom<Val> for $type {
            type Error = TypeError;

            fn try_from(value: Val) -> Result<Self, Self::Error> {
                if let Val::$variant(value) = value {
                    Ok(value)
                } else {
                    Err(TypeError::new(concat!("expected ", $wit)))
                }
            }
        }
    };
}

primitive_conversion!(bool, Bool, "bool");
primitive_conversion!(i8, S8, "s8");
primitive_conversion!(u8, U8, "u8");
primitive_conversion!(i16, S16, "s16");
primitive_conversion!(u16, U16, "u16");
primitive_conversion!(i32, S32, "s32");
primitive_conversion!(u32, U32, "u32");
primitive_conversion!(i64, S64, "s64");
primitive_conversion!(u64, U64, "u64");
primitive_conversion!(f32, F32, "f32");
primitive_conversion!(f64, F64, "f64");
primitive_conversion!(char, Char, "char");
primitive_conversion!(String, String, "string");

impl From<&str> for Val {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<Vec<u8>> for Val {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl TryFrom<Val> for Vec<u8> {
    type Error = TypeError;

    fn try_from(value: Val) -> Result<Self, Self::Error> {
        if let Val::Bytes(value) = value {
            Ok(value)
        } else {
            Err(TypeError::new("expected list<u8>"))
        }
    }
}

/// An error converting an engine-neutral value to its expected WIT type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError(String);

impl TypeError {
    /// Creates an error describing why a value did not have the expected structure.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl Display for TypeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}

impl Error for TypeError {}

#[cfg(test)]
mod tests {
    use super::{TypeError, Val};

    #[test]
    fn bytes_are_the_only_byte_list_shape() {
        let bytes = vec![0, 127, 255];
        assert_eq!(Vec::<u8>::try_from(Val::from(bytes.clone())), Ok(bytes));
        assert_eq!(
            Vec::<u8>::try_from(Val::List(vec![Val::U8(1)])),
            Err(TypeError::new("expected list<u8>"))
        );
    }
}
