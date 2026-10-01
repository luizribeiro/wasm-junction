use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Type, TypeDefKind, TypeId, TypeOwner};

use super::{Generator, rust_ident};

impl Generator<'_> {
    pub(super) fn rust_type(&self, ty: Type, item: &str) -> syn::Result<TokenStream> {
        match ty {
            Type::Bool => Ok(quote!(bool)),
            Type::U8 => Ok(quote!(u8)),
            Type::U16 => Ok(quote!(u16)),
            Type::U32 => Ok(quote!(u32)),
            Type::U64 => Ok(quote!(u64)),
            Type::S8 => Ok(quote!(i8)),
            Type::S16 => Ok(quote!(i16)),
            Type::S32 => Ok(quote!(i32)),
            Type::S64 => Ok(quote!(i64)),
            Type::F32 => Ok(quote!(f32)),
            Type::F64 => Ok(quote!(f64)),
            Type::Char => Ok(quote!(char)),
            Type::String => Ok(quote!(::std::string::String)),
            Type::Id(id) => {
                if self.resolve.types[id].name.is_some() {
                    self.named_type(id)
                } else {
                    self.type_kind(&self.resolve.types[id].kind, item)
                }
            }
            Type::ErrorContext => Err(Self::unsupported(item, "error-context")),
        }
    }

    pub(super) fn type_kind(&self, kind: &TypeDefKind, item: &str) -> syn::Result<TokenStream> {
        match kind {
            TypeDefKind::Type(ty) => self.rust_type(*ty, item),
            TypeDefKind::List(ty) => {
                let ty = self.rust_type(*ty, item)?;
                Ok(quote!(::std::vec::Vec<#ty>))
            }
            TypeDefKind::Option(ty) => {
                let ty = self.rust_type(*ty, item)?;
                Ok(quote!(::std::option::Option<#ty>))
            }
            TypeDefKind::Result(result) => {
                let ok = self.optional_type(result.ok, item)?;
                let err = self.optional_type(result.err, item)?;
                Ok(quote!(::std::result::Result<#ok, #err>))
            }
            TypeDefKind::Tuple(tuple) => {
                let types = tuple
                    .types
                    .iter()
                    .map(|ty| self.rust_type(*ty, item))
                    .collect::<syn::Result<Vec<_>>>()?;
                Ok(quote!((#(#types,)*)))
            }
            TypeDefKind::Resource | TypeDefKind::Handle(_) => Ok(quote!(::wasm_junction::Resource)),
            other => Err(Self::unsupported(item, other.as_str())),
        }
    }

    pub(super) fn named_type(&self, id: TypeId) -> syn::Result<TokenStream> {
        let definition = &self.resolve.types[id];
        let name = definition
            .name
            .as_deref()
            .ok_or_else(|| Self::unsupported("anonymous type", definition.kind.as_str()))?;
        let ident = rust_ident(&name.to_upper_camel_case())?;
        let TypeOwner::Interface(owner) = definition.owner else {
            return Err(Self::unsupported(name, "world type"));
        };
        let interface = self.resolve.interfaces[owner]
            .name
            .as_deref()
            .ok_or_else(|| Self::unsupported(name, "inline interface type"))?;
        let module = rust_ident(&interface.to_snake_case())?;
        let package = self.resolve.interfaces[owner]
            .package
            .ok_or_else(|| Self::unsupported(name, "package-less interface type"))?;
        if let Some(path) = self.with.get(&package) {
            Ok(quote!(#path::#module::#ident))
        } else {
            Ok(quote!(super::#module::#ident))
        }
    }

    fn optional_type(&self, ty: Option<Type>, item: &str) -> syn::Result<TokenStream> {
        ty.map_or_else(|| Ok(quote!(())), |ty| self.rust_type(ty, item))
    }

    pub(super) fn unsupported(item: &str, shape: &str) -> syn::Error {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("WIT {shape} in `{item}` is not supported yet"),
        )
    }
}
