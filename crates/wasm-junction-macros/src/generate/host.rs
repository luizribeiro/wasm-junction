use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::Function;

use super::{Generator, rust_ident};

impl Generator<'_> {
    pub(super) fn host_trait<'a>(
        &self,
        interface: &str,
        functions: impl Iterator<Item = &'a Function> + Clone,
    ) -> syn::Result<TokenStream> {
        let methods = functions
            .clone()
            .map(|function| self.host_method(function, false))
            .collect::<syn::Result<Vec<_>>>()?;
        let forwarding = functions
            .map(|function| self.host_method(function, true))
            .collect::<syn::Result<Vec<_>>>()?;
        Ok(quote! {
            #[doc = concat!("A host implementation of the `", #interface, "` interface.")]
            pub trait Host: ::wasm_junction::HostBound + Sized + 'static {
                #(#methods)*
            }

            impl<T: Host> Host for ::std::sync::Arc<T> {
                #(#forwarding)*
            }
        })
    }

    fn host_method(&self, function: &Function, forward: bool) -> syn::Result<TokenStream> {
        let wit_name = &function.name;
        let method = rust_ident(&wit_name.to_snake_case())?;
        let parameters = function
            .params
            .iter()
            .map(|param| rust_ident(&param.name.to_snake_case()))
            .collect::<syn::Result<Vec<_>>>()?;
        let parameter_types = function
            .params
            .iter()
            .map(|param| self.rust_type(param.ty, wit_name))
            .collect::<syn::Result<Vec<_>>>()?;
        let output = function
            .result
            .map_or_else(|| Ok(quote!(())), |ty| self.rust_type(ty, wit_name))?;
        let return_type = if function.kind.is_async() {
            quote!(impl ::std::future::Future<Output = #output> + ::wasm_junction::MaybeSend)
        } else {
            output
        };
        let body = if forward {
            let call = quote!(self.as_ref().#method(cx, #(#parameters),*));
            if function.kind.is_async() {
                quote!({ async move { #call.await } })
            } else {
                quote!({ #call })
            }
        } else {
            quote!(;)
        };
        let docs = (!forward)
            .then(|| quote!(#[doc = concat!("Implements the WIT `", #wit_name, "` function.")]));
        Ok(quote! {
            #docs
            fn #method(
                &self,
                cx: &::wasm_junction::CallContext,
                #(#parameters: #parameter_types),*
            ) -> #return_type #body
        })
    }
}
