use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Enum, Flags, Record, Type, TypeDefKind, Variant};

use super::{Generator, rust_ident};

impl Generator<'_> {
    pub(super) fn variant(&self, name: &str, variant: &Variant) -> syn::Result<TokenStream> {
        let type_ident = rust_ident(&name.to_upper_camel_case())?;
        let cases = variant
            .cases
            .iter()
            .map(|case| {
                let ident = rust_ident(&case.name.to_upper_camel_case())?;
                let payload = case
                    .ty
                    .map(|ty| self.rust_type(ty, name))
                    .transpose()?
                    .map(|ty| quote!((#ty)));
                let wit_name = &case.name;
                Ok(quote! {
                    #[doc = concat!("The WIT `", #wit_name, "` case.")]
                    #ident #payload
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let encode = variant
            .cases
            .iter()
            .map(|case| {
                let ident = rust_ident(&case.name.to_upper_camel_case())?;
                let wit_name = &case.name;
                if let Some(ty) = case.ty {
                    let value = self.encode(ty, quote!(value), name)?;
                    Ok(
                        quote!(#type_ident::#ident(value) => ::wasm_junction::Val::Variant {
                            case: #wit_name.to_owned(),
                            value: Some(::std::boxed::Box::new(#value)),
                        }),
                    )
                } else {
                    Ok(
                        quote!(#type_ident::#ident => ::wasm_junction::Val::Variant {
                            case: #wit_name.to_owned(), value: None,
                        }),
                    )
                }
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let decode = variant
            .cases
            .iter()
            .map(|case| {
                let ident = rust_ident(&case.name.to_upper_camel_case())?;
                let wit_name = &case.name;
                if let Some(ty) = case.ty {
                    let value = self.decode(ty, quote!(*value), name)?;
                    Ok(quote!((#wit_name, Some(value)) => #value.map(Self::#ident)))
                } else {
                    Ok(quote!((#wit_name, None) => Ok(Self::#ident)))
                }
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let eq_hash = variant
            .cases
            .iter()
            .all(|case| case.ty.is_none_or(|ty| self.has_eq(ty)))
            .then(|| quote!(Eq, Hash,));
        let expected = format!("expected {name} variant");
        Ok(quote! {
            #[doc = concat!("The WIT `", #name, "` variant.")]
            #[derive(Debug, Clone, PartialEq, #eq_hash)]
            pub enum #type_ident { #(#cases,)* }

            impl ::std::convert::From<#type_ident> for ::wasm_junction::Val {
                fn from(value: #type_ident) -> Self { match value { #(#encode,)* } }
            }

            impl ::std::convert::TryFrom<::wasm_junction::Val> for #type_ident {
                type Error = ::wasm_junction::TypeError;

                fn try_from(value: ::wasm_junction::Val) -> ::std::result::Result<Self, Self::Error> {
                    let ::wasm_junction::Val::Variant { case, value } = value else {
                        return Err(::wasm_junction::TypeError::new(#expected));
                    };
                    match (case.as_str(), value) {
                        #(#decode,)*
                        _ => Err(::wasm_junction::TypeError::new(#expected)),
                    }
                }
            }
        })
    }

    pub(super) fn flags(name: &str, flags: &Flags) -> syn::Result<TokenStream> {
        let type_ident = rust_ident(&name.to_upper_camel_case())?;
        let fields = flags
            .flags
            .iter()
            .map(|flag| rust_ident(&flag.name.to_snake_case()))
            .collect::<syn::Result<Vec<_>>>()?;
        let wit_names = flags
            .flags
            .iter()
            .map(|flag| &flag.name)
            .collect::<Vec<_>>();
        let decode = wit_names.iter().zip(&fields).map(|(name, field)| {
            let duplicate = format!("duplicate {name} flag");
            quote!(#name => {
                if result.#field {
                    return Err(::wasm_junction::TypeError::new(#duplicate));
                }
                result.#field = true;
            })
        });
        let expected = format!("expected {name} flags");
        Ok(quote! {
            #[doc = concat!("The WIT `", #name, "` flags.")]
            #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
            pub struct #type_ident {
                #(#[doc = concat!("Whether the WIT `", #wit_names, "` flag is active.")]
                  pub #fields: bool,)*
            }

            impl ::std::convert::From<#type_ident> for ::wasm_junction::Val {
                fn from(value: #type_ident) -> Self {
                    let #type_ident { #(#fields,)* } = value;
                    let mut flags = ::std::vec::Vec::new();
                    #(if #fields { flags.push(#wit_names.to_owned()); })*
                    Self::Flags(flags)
                }
            }

            impl ::std::convert::TryFrom<::wasm_junction::Val> for #type_ident {
                type Error = ::wasm_junction::TypeError;

                fn try_from(value: ::wasm_junction::Val) -> ::std::result::Result<Self, Self::Error> {
                    let ::wasm_junction::Val::Flags(flags) = value else {
                        return Err(::wasm_junction::TypeError::new(#expected));
                    };
                    let mut result = Self::default();
                    for flag in flags {
                        match flag.as_str() {
                            #(#decode,)*
                            _ => return Err(::wasm_junction::TypeError::new(#expected)),
                        }
                    }
                    Ok(result)
                }
            }
        })
    }

    pub(super) fn enum_(name: &str, enum_: &Enum) -> syn::Result<TokenStream> {
        let type_ident = rust_ident(&name.to_upper_camel_case())?;
        let cases = enum_
            .cases
            .iter()
            .map(|case| rust_ident(&case.name.to_upper_camel_case()))
            .collect::<syn::Result<Vec<_>>>()?;
        let wit_cases = enum_.cases.iter().map(|case| &case.name);
        let encode_cases = enum_.cases.iter().zip(&cases).map(|(case, case_ident)| {
            let name = &case.name;
            quote!(#type_ident::#case_ident => #name)
        });
        let decode_cases = enum_.cases.iter().zip(&cases).map(|(case, ident)| {
            let name = &case.name;
            quote!(#name => Ok(Self::#ident))
        });
        let expected = format!("expected {name} enum");
        Ok(quote! {
            #[doc = concat!("The WIT `", #name, "` enum.")]
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub enum #type_ident {
                #(#[doc = concat!("The WIT `", #wit_cases, "` case.")] #cases,)*
            }

            impl ::std::convert::From<#type_ident> for ::wasm_junction::Val {
                fn from(value: #type_ident) -> Self {
                    Self::Enum(match value { #(#encode_cases,)* }.to_owned())
                }
            }

            impl ::std::convert::TryFrom<::wasm_junction::Val> for #type_ident {
                type Error = ::wasm_junction::TypeError;

                fn try_from(value: ::wasm_junction::Val) -> ::std::result::Result<Self, Self::Error> {
                    let ::wasm_junction::Val::Enum(case) = value else {
                        return Err(::wasm_junction::TypeError::new(#expected));
                    };
                    match case.as_str() {
                        #(#decode_cases,)*
                        _ => Err(::wasm_junction::TypeError::new(#expected)),
                    }
                }
            }
        })
    }

    pub(super) fn record(&self, name: &str, record: &Record) -> syn::Result<TokenStream> {
        let ident = rust_ident(&name.to_upper_camel_case())?;
        let fields = record
            .fields
            .iter()
            .map(|field| {
                let field_name = rust_ident(&field.name.to_snake_case())?;
                let ty = self.rust_type(field.ty, name)?;
                let wit_name = &field.name;
                Ok(quote! {
                    #[doc = concat!("The record's `", #wit_name, "` field.")]
                    pub #field_name: #ty
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let eq_hash = record
            .fields
            .iter()
            .all(|field| self.has_eq(field.ty))
            .then(|| quote!(Eq, Hash,));
        let field_names = record
            .fields
            .iter()
            .map(|field| rust_ident(&field.name.to_snake_case()))
            .collect::<syn::Result<Vec<_>>>()?;
        let values = record
            .fields
            .iter()
            .zip(&field_names)
            .map(|(field, field_name)| {
                let wit_name = &field.name;
                let value = self.encode(field.ty, quote!(#field_name), name)?;
                Ok(quote!((#wit_name.to_owned(), #value)))
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let decoded = record
            .fields
            .iter()
            .zip(&field_names)
            .map(|(field, field_name)| {
                let wit_name = &field.name;
                let missing = format!("{name}.{wit_name} is missing");
                let wrong = format!("expected {name}.{wit_name} field");
                let ty = self.rust_type(field.ty, name)?;
                let value = self.decode(field.ty, quote!(value), name)?;
                Ok(quote! {
                    let #field_name: #ty = {
                        let Some((field, value)) = __wasm_junction_fields.next() else {
                            return Err(::wasm_junction::TypeError::new(#missing));
                        };
                        if field != #wit_name {
                            return Err(::wasm_junction::TypeError::new(#wrong));
                        }
                        #value?
                    };
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let extra = format!("{name} has unexpected fields");
        Ok(quote! {
            #[doc = concat!("The WIT `", #name, "` record.")]
            #[derive(Debug, Clone, PartialEq, #eq_hash)]
            pub struct #ident { #(#fields,)* }

            impl ::std::convert::From<#ident> for ::wasm_junction::Val {
                fn from(value: #ident) -> Self {
                    let #ident { #(#field_names,)* } = value;
                    Self::Record(::std::vec![#(#values,)*])
                }
            }

            impl ::std::convert::TryFrom<::wasm_junction::Val> for #ident {
                type Error = ::wasm_junction::TypeError;

                fn try_from(value: ::wasm_junction::Val) -> ::std::result::Result<Self, Self::Error> {
                    let ::wasm_junction::Val::Record(fields) = value else {
                        return Err(::wasm_junction::TypeError::new(concat!("expected ", #name, " record")));
                    };
                    let mut __wasm_junction_fields = fields.into_iter();
                    #(#decoded)*
                    if __wasm_junction_fields.next().is_some() {
                        return Err(::wasm_junction::TypeError::new(#extra));
                    }
                    Ok(Self { #(#field_names,)* })
                }
            }
        })
    }

    fn has_eq(&self, ty: Type) -> bool {
        match ty {
            Type::F32 | Type::F64 | Type::ErrorContext => false,
            Type::Id(id) => match &self.resolve.types[id].kind {
                TypeDefKind::Record(record) => {
                    record.fields.iter().all(|field| self.has_eq(field.ty))
                }
                TypeDefKind::Variant(variant) => variant
                    .cases
                    .iter()
                    .all(|case| case.ty.is_none_or(|ty| self.has_eq(ty))),
                TypeDefKind::Tuple(tuple) => tuple.types.iter().all(|ty| self.has_eq(*ty)),
                TypeDefKind::Option(ty) | TypeDefKind::List(ty) | TypeDefKind::Type(ty) => {
                    self.has_eq(*ty)
                }
                TypeDefKind::Result(result) => {
                    result.ok.is_none_or(|ty| self.has_eq(ty))
                        && result.err.is_none_or(|ty| self.has_eq(ty))
                }
                TypeDefKind::Enum(_) | TypeDefKind::Flags(_) => true,
                _ => false,
            },
            _ => true,
        }
    }
}
