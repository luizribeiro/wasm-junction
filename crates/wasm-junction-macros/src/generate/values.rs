use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Type, TypeDefKind};

use super::{Generator, rust_ident};

impl Generator<'_> {
    pub(super) fn encode(
        &self,
        ty: Type,
        value: TokenStream,
        item: &str,
    ) -> syn::Result<TokenStream> {
        match ty {
            Type::Id(id) => match &self.resolve.types[id].kind {
                TypeDefKind::Record(_)
                | TypeDefKind::Variant(_)
                | TypeDefKind::Enum(_)
                | TypeDefKind::Flags(_) => {
                    Ok(quote!(::std::convert::Into::<::wasm_junction::Val>::into(#value)))
                }
                kind => self.encode_kind(kind, value, item),
            },
            Type::ErrorContext => Err(Self::unsupported(item, "error-context")),
            _ => Ok(quote!(::wasm_junction::Val::from(#value))),
        }
    }

    fn encode_kind(
        &self,
        kind: &TypeDefKind,
        value: TokenStream,
        item: &str,
    ) -> syn::Result<TokenStream> {
        match kind {
            TypeDefKind::Type(ty) => self.encode(*ty, value, item),
            TypeDefKind::List(ty) => {
                let element = self.encode(*ty, quote!(value), item)?;
                Ok(quote!(::wasm_junction::Val::List(
                    #value.into_iter().map(|value| #element).collect()
                )))
            }
            TypeDefKind::Option(ty) => {
                let payload = self.encode(*ty, quote!(value), item)?;
                Ok(quote!(::wasm_junction::Val::Option(
                    #value.map(|value| ::std::boxed::Box::new(#payload))
                )))
            }
            TypeDefKind::Result(result) => {
                let ok = self.result_payload(result.ok, true, item)?;
                let err = self.result_payload(result.err, false, item)?;
                Ok(quote!(match #value { #ok, #err }))
            }
            TypeDefKind::Tuple(tuple) => {
                let names = (0..tuple.types.len())
                    .map(|index| rust_ident(&format!("value_{index}")))
                    .collect::<syn::Result<Vec<_>>>()?;
                let values = tuple
                    .types
                    .iter()
                    .zip(&names)
                    .map(|(ty, name)| self.encode(*ty, quote!(#name), item))
                    .collect::<syn::Result<Vec<_>>>()?;
                Ok(quote!({
                    let (#(#names,)*) = #value;
                    ::wasm_junction::Val::Tuple(::std::vec![#(#values,)*])
                }))
            }
            other => Err(Self::unsupported(item, other.as_str())),
        }
    }

    fn result_payload(&self, ty: Option<Type>, ok: bool, item: &str) -> syn::Result<TokenStream> {
        let constructor = if ok { quote!(Ok) } else { quote!(Err) };
        if let Some(ty) = ty {
            let value = self.encode(ty, quote!(value), item)?;
            Ok(quote!(#constructor(value) => ::wasm_junction::Val::Result(
                #constructor(Some(::std::boxed::Box::new(#value)))
            )))
        } else {
            Ok(quote!(#constructor(()) => ::wasm_junction::Val::Result(#constructor(None))))
        }
    }
}
