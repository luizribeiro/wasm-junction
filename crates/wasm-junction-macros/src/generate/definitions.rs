use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Record, Type, TypeDefKind};

use super::{Generator, rust_ident};

impl Generator<'_> {
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
                let value = self.decode(field.ty, quote!(value), name)?;
                Ok(quote! {
                    let Some((field, value)) = fields.next() else {
                        return Err(::wasm_junction::TypeError::new(#missing));
                    };
                    if field != #wit_name {
                        return Err(::wasm_junction::TypeError::new(#wrong));
                    }
                    let #field_name = #value?;
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
                    let mut fields = fields.into_iter();
                    #(#decoded)*
                    if fields.next().is_some() {
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
