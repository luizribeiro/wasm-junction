use wasmtime::component::{ResourceAny, Type};

use crate::stream_values::StreamValue;

mod compound;
#[cfg(test)]
pub(crate) mod test_support;
#[allow(dead_code)]
mod validate;

use compound::{WrapList, WrapOption};
#[allow(unused_imports)]
pub(crate) use validate::validate_component_streams;

pub(crate) trait StreamTypeVisitor {
    type Output;

    fn visit<T: StreamValue>(self) -> Result<Self::Output, wasmtime::Error>;
}

pub(crate) fn visit_stream_type<V: StreamTypeVisitor>(
    ty: &Type,
    visitor: V,
) -> Result<V::Output, wasmtime::Error> {
    D2::resolve(ty, visitor)
}

pub(super) trait Depth {
    fn resolve<V: StreamTypeVisitor>(ty: &Type, visitor: V) -> Result<V::Output, wasmtime::Error>;
}

pub(super) struct D0;
struct D1;
struct D2;

impl Depth for D0 {
    fn resolve<V: StreamTypeVisitor>(ty: &Type, visitor: V) -> Result<V::Output, wasmtime::Error> {
        match ty {
            Type::Bool => visitor.visit::<bool>(),
            Type::S8 => visitor.visit::<i8>(),
            Type::U8 => visitor.visit::<u8>(),
            Type::S16 => visitor.visit::<i16>(),
            Type::U16 => visitor.visit::<u16>(),
            Type::S32 => visitor.visit::<i32>(),
            Type::U32 => visitor.visit::<u32>(),
            Type::S64 => visitor.visit::<i64>(),
            Type::U64 => visitor.visit::<u64>(),
            Type::Float32 => visitor.visit::<f32>(),
            Type::Float64 => visitor.visit::<f64>(),
            Type::Char => visitor.visit::<char>(),
            Type::String => visitor.visit::<String>(),
            Type::Own(_) | Type::Borrow(_) => visitor.visit::<ResourceAny>(),
            _ => Err(unsupported(ty)),
        }
    }
}

macro_rules! depth {
    ($current:ty, $lower:ty) => {
        impl Depth for $current {
            fn resolve<V: StreamTypeVisitor>(
                ty: &Type,
                visitor: V,
            ) -> Result<V::Output, wasmtime::Error> {
                match ty {
                    Type::List(ty) => <$lower>::resolve(&ty.ty(), WrapList(visitor)),
                    Type::Option(ty) => <$lower>::resolve(&ty.ty(), WrapOption(visitor)),
                    _ => D0::resolve(ty, visitor),
                }
            }
        }
    };
}

depth!(D1, D0);
depth!(D2, D1);

fn unsupported(ty: &Type) -> wasmtime::Error {
    wasmtime::Error::new(wasm_junction_core::CallError::refused(format!(
        "stream item type `{}` is not supported on Wasmtime; supported item types are scalar values, strings, resources, and lists or options nested up to two layers",
        type_name(ty),
    )))
}

fn type_name(ty: &Type) -> &'static str {
    match ty {
        Type::Record(_) => "record",
        Type::Variant(_) => "variant",
        Type::Enum(_) => "enum",
        Type::Flags(_) => "flags",
        Type::List(_) => "list",
        Type::Option(_) => "option",
        Type::Result(_) => "result",
        Type::Tuple(_) => "tuple",
        _ => "value",
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::item_type;
    use super::*;

    struct RustType;

    impl StreamTypeVisitor for RustType {
        type Output = &'static str;

        fn visit<T: StreamValue>(self) -> Result<Self::Output, wasmtime::Error> {
            Ok(std::any::type_name::<T>())
        }
    }

    #[test]
    fn resolver_selects_leaf_and_nested_static_types() {
        assert_eq!(
            visit_stream_type(&item_type("", "u32"), RustType).unwrap(),
            std::any::type_name::<u32>()
        );
        assert_eq!(
            visit_stream_type(&item_type("", "list<option<string>>"), RustType).unwrap(),
            std::any::type_name::<Vec<Option<String>>>()
        );
        assert_eq!(
            visit_stream_type(&item_type("", "option<list<u8>>"), RustType).unwrap(),
            std::any::type_name::<Option<Vec<u8>>>()
        );
    }
}
