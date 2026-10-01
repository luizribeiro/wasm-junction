use heck::ToSnakeCase;
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use wit_parser::{InterfaceId, PackageId, Resolve};

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
        Ok(quote! {
            #[doc = concat!("Bindings for the `", #interface, "` interface.")]
            pub mod #module {
                /// The fully qualified WIT interface name.
                pub const INTERFACE: &str = #interface;
            }
        })
    }
}

pub(crate) fn rust_ident(name: &str) -> syn::Result<Ident> {
    syn::parse_str(name)
        .or_else(|_| syn::parse_str(&format!("r#{name}")))
        .or_else(|_| syn::parse_str(&format!("{name}_")))
        .map_err(|_| syn::Error::new(Span::call_site(), format!("invalid Rust name `{name}`")))
}
