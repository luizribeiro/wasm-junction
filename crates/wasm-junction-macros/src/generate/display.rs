use heck::ToUpperCamelCase;
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use wit_parser::{Type, TypeDefKind, TypeId};

use super::{Generator, rust_ident};

impl Generator<'_> {
    pub(super) fn error_display(
        &self,
        name: &str,
        id: TypeId,
        ident: &Ident,
    ) -> syn::Result<TokenStream> {
        if !self.errors.contains(&id) {
            return Ok(TokenStream::new());
        }
        let (rule, body) = match &self.resolve.types[id].kind {
            TypeDefKind::Enum(enum_) => {
                let arms = enum_
                    .cases
                    .iter()
                    .map(|case| {
                        let ident = rust_ident(&case.name.to_upper_camel_case())?;
                        let name = &case.name;
                        Ok(quote!(Self::#ident => formatter.write_str(#name)))
                    })
                    .collect::<syn::Result<Vec<_>>>()?;
                (
                    "Displays the WIT case name.",
                    quote!(match self { #(#arms,)* }),
                )
            }
            TypeDefKind::Variant(variant) => {
                let arms = variant.cases.iter().map(|case| {
                    let ident = rust_ident(&case.name.to_upper_camel_case())?;
                    let name = &case.name;
                    if let Some(ty) = case.ty {
                        let marker = if self.has_display(ty) { quote!("{}") } else { quote!("{:?}") };
                        Ok(quote!(Self::#ident(value) => write!(formatter, concat!(#name, ": ", #marker), value)))
                    } else {
                        Ok(quote!(Self::#ident => formatter.write_str(#name)))
                    }
                }).collect::<syn::Result<Vec<_>>>()?;
                (
                    "Displays the WIT case name and any payload as a message.",
                    quote!(match self { #(#arms,)* }),
                )
            }
            _ => (
                "Displays the WIT type name and compact debug representation.",
                quote!(write!(formatter, concat!(#name, ": {:?}"), self)),
            ),
        };
        Ok(quote! {
            #[doc = #rule]
            impl ::std::fmt::Display for #ident {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    #body
                }
            }
            impl ::std::error::Error for #ident {}
        })
    }

    fn has_display(&self, ty: Type) -> bool {
        match ty {
            Type::Id(id) => match self.resolve.types[id].kind {
                TypeDefKind::Type(ty) => self.has_display(ty),
                _ => self.errors.contains(&id),
            },
            Type::ErrorContext => false,
            _ => true,
        }
    }
}
