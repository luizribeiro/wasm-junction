use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use wit_parser::{InterfaceId, PackageId, Resolve};

mod collisions;
mod definitions;
mod errors;
mod reachability;
mod types;
mod validate;
mod values;
mod walk;

pub(crate) fn generate(
    resolve: &Resolve,
    package_id: PackageId,
    span: Span,
) -> syn::Result<TokenStream> {
    validate::package(resolve, package_id, span)?;
    collisions::check(resolve, package_id, span)?;
    let selected = reachability::find(resolve, package_id)?;
    let generator = Generator {
        resolve,
        errors: errors::find(resolve, package_id)?,
        selected,
    };
    let modules = resolve
        .interfaces
        .iter()
        .filter(|(id, _)| generator.selected.contains_key(id))
        .map(|(id, interface)| {
            let name = interface.name.as_deref().ok_or_else(|| {
                syn::Error::new(Span::call_site(), "selected interface has no name")
            })?;
            generator.interface(name, id)
        })
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(quote!(#(#modules)*))
}

struct Generator<'a> {
    resolve: &'a Resolve,
    errors: std::collections::HashSet<wit_parser::TypeId>,
    selected: std::collections::HashMap<InterfaceId, std::collections::HashSet<wit_parser::TypeId>>,
}

impl Generator<'_> {
    fn interface(&self, name: &str, id: InterfaceId) -> syn::Result<TokenStream> {
        let module = rust_ident(&name.to_snake_case())?;
        let package_id = self.resolve.interfaces[id]
            .package
            .ok_or_else(|| syn::Error::new(Span::call_site(), "interface has no package"))?;
        let interface = self.resolve.packages[package_id].name.interface_id(name);
        let types = self.resolve.interfaces[id]
            .types
            .iter()
            .filter(|(_, type_id)| self.selected[&id].contains(*type_id))
            .map(|(export, type_id)| {
                let definition = &self.resolve.types[*type_id];
                if definition.owner == wit_parser::TypeOwner::Interface(id) {
                    self.type_definition(export, *type_id)
                } else {
                    let name = rust_ident(&export.to_upper_camel_case())?;
                    let target = self.named_type(*type_id)?;
                    Ok(quote!(pub use #target as #name;))
                }
            })
            .collect::<syn::Result<Vec<_>>>()?;
        Ok(quote! {
            #[doc = concat!("Bindings for the `", #interface, "` interface.")]
            pub mod #module {
                /// The fully qualified WIT interface name.
                pub const INTERFACE: &str = #interface;
                #(#types)*
            }
        })
    }

    fn type_definition(&self, name: &str, id: wit_parser::TypeId) -> syn::Result<TokenStream> {
        let ident = rust_ident(&name.to_upper_camel_case())?;
        let (definition, nominal) = match &self.resolve.types[id].kind {
            wit_parser::TypeDefKind::Record(record) => (self.record(name, record)?, true),
            wit_parser::TypeDefKind::Enum(enum_) => (Self::enum_(name, enum_)?, true),
            wit_parser::TypeDefKind::Flags(flags) => (Self::flags(name, flags)?, true),
            wit_parser::TypeDefKind::Variant(variant) => (self.variant(name, variant)?, true),
            kind => {
                let ty = self.type_kind(kind, name)?;
                (
                    quote! {
                        #[doc = concat!("The WIT `", #name, "` type.")]
                        pub type #ident = #ty;
                    },
                    false,
                )
            }
        };
        let error = nominal.then(|| self.error_impl(name, id, &ident));
        Ok(quote!(#definition #error))
    }

    fn error_impl(&self, name: &str, id: wit_parser::TypeId, ident: &Ident) -> TokenStream {
        if !self.errors.contains(&id) {
            return TokenStream::new();
        }
        quote! {
            #[doc = "Formats the WIT type name followed by its structured debug form."]
            impl ::std::fmt::Display for #ident {
                fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    write!(formatter, concat!(#name, ": {:?}"), self)
                }
            }

            impl ::std::error::Error for #ident {}
        }
    }
}

pub(crate) fn rust_ident(name: &str) -> syn::Result<Ident> {
    syn::parse_str(name)
        .or_else(|_| syn::parse_str(&format!("r#{name}")))
        .or_else(|_| syn::parse_str(&format!("{name}_")))
        .map_err(|_| syn::Error::new(Span::call_site(), format!("invalid Rust name `{name}`")))
}
