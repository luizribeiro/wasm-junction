use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Type, TypeDefKind};

use super::{Generator, rust_ident};

impl Generator<'_> {
    pub(super) fn decode(
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
                    let ty = self.named_type(id)?;
                    Ok(
                        quote!(<#ty as ::std::convert::TryFrom<::wasm_junction::Val>>::try_from(#value)),
                    )
                }
                TypeDefKind::Resource | TypeDefKind::Handle(_) => Ok(quote!(match #value {
                    ::wasm_junction::Val::Resource(value) => Ok(value),
                    _ => Err(::wasm_junction::TypeError::new("expected resource")),
                })),
                TypeDefKind::Stream(Some(Type::U8)) => Ok(quote!(match #value {
                    ::wasm_junction::Val::Stream(value) => Ok(value),
                    _ => Err(::wasm_junction::TypeError::new("expected stream<u8>")),
                })),
                kind => self.decode_kind(kind, value, item),
            },
            Type::ErrorContext => Err(Self::unsupported(item, "error-context")),
            _ => Ok(quote!(::std::convert::TryFrom::try_from(#value))),
        }
    }

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
                TypeDefKind::Resource | TypeDefKind::Handle(_) => {
                    Ok(quote!(::wasm_junction::Val::Resource(#value)))
                }
                TypeDefKind::Stream(Some(Type::U8)) => {
                    Ok(quote!(::wasm_junction::Val::Stream(#value)))
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

    fn decode_kind(
        &self,
        kind: &TypeDefKind,
        value: TokenStream,
        item: &str,
    ) -> syn::Result<TokenStream> {
        let expected = format!("expected {item} {}", kind.as_str());
        match kind {
            TypeDefKind::Type(ty) => self.decode(*ty, value, item),
            TypeDefKind::List(ty) => {
                let element = self.decode(*ty, quote!(value), item)?;
                Ok(quote!(match #value {
                    ::wasm_junction::Val::List(values) => values.into_iter()
                        .map(|value| #element).collect(),
                    _ => Err(::wasm_junction::TypeError::new(#expected)),
                }))
            }
            TypeDefKind::Option(ty) => {
                let payload = self.decode(*ty, quote!(*value), item)?;
                Ok(quote!(match #value {
                    ::wasm_junction::Val::Option(value) => value.map(|value| #payload).transpose(),
                    _ => Err(::wasm_junction::TypeError::new(#expected)),
                }))
            }
            TypeDefKind::Result(result) => {
                let ok = self.decode_result_payload(result.ok, true, item)?;
                let err = self.decode_result_payload(result.err, false, item)?;
                let missing = format!("{item} result payload does not match its type");
                Ok(quote!(match #value {
                    ::wasm_junction::Val::Result(value) => match value {
                        #ok, #err,
                        _ => Err(::wasm_junction::TypeError::new(#missing)),
                    },
                    _ => Err(::wasm_junction::TypeError::new(#expected)),
                }))
            }
            TypeDefKind::Tuple(tuple) => {
                let names = (0..tuple.types.len())
                    .map(|index| rust_ident(&format!("value_{index}")))
                    .collect::<syn::Result<Vec<_>>>()?;
                let len = names.len();
                let values = tuple
                    .types
                    .iter()
                    .zip(&names)
                    .map(|(ty, name)| {
                        let value = self.decode(*ty, quote!(#name), item)?;
                        Ok(quote!(#value?))
                    })
                    .collect::<syn::Result<Vec<_>>>()?;
                Ok(quote!((|| {
                    let ::wasm_junction::Val::Tuple(values) = #value else {
                        return Err(::wasm_junction::TypeError::new(#expected));
                    };
                    let [#(#names,)*] = <[::wasm_junction::Val; #len]>::try_from(values)
                        .map_err(|_| ::wasm_junction::TypeError::new(#expected))?;
                    Ok((#(#values,)*))
                })()))
            }
            other => Err(Self::unsupported(item, other.as_str())),
        }
    }

    fn decode_result_payload(
        &self,
        ty: Option<Type>,
        ok: bool,
        item: &str,
    ) -> syn::Result<TokenStream> {
        let constructor = if ok { quote!(Ok) } else { quote!(Err) };
        if let Some(ty) = ty {
            let value = self.decode(ty, quote!(*value), item)?;
            Ok(quote!(#constructor(Some(value)) => #value.map(#constructor)))
        } else {
            Ok(quote!(#constructor(None) => Ok(#constructor(()))))
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
