use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use wit_parser::{InterfaceId, PackageId, Resolve};

mod definitions;
mod types;
mod values;

pub(crate) fn generate(resolve: &Resolve, package_id: PackageId) -> syn::Result<TokenStream> {
    let package = &resolve.packages[package_id];
    let generator = Generator { resolve };
    let modules = package
        .interfaces
        .iter()
        .map(|(name, id)| generator.interface(name, *id))
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(quote!(#(#modules)*))
}

struct Generator<'a> {
    resolve: &'a Resolve,
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
        match &self.resolve.types[id].kind {
            wit_parser::TypeDefKind::Record(record) => return self.record(name, record),
            wit_parser::TypeDefKind::Enum(enum_) => return Self::enum_(name, enum_),
            wit_parser::TypeDefKind::Flags(flags) => return Self::flags(name, flags),
            wit_parser::TypeDefKind::Variant(variant) => return self.variant(name, variant),
            _ => {}
        }
        let ident = rust_ident(&name.to_upper_camel_case())?;
        let ty = self.type_kind(&self.resolve.types[id].kind, name)?;
        Ok(quote! {
            #[doc = concat!("The WIT `", #name, "` type.")]
            pub type #ident = #ty;
        })
    }
}

pub(crate) fn rust_ident(name: &str) -> syn::Result<Ident> {
    syn::parse_str(name)
        .or_else(|_| syn::parse_str(&format!("r#{name}")))
        .or_else(|_| syn::parse_str(&format!("{name}_")))
        .map_err(|_| syn::Error::new(Span::call_site(), format!("invalid Rust name `{name}`")))
}
