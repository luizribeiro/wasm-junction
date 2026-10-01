use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::Function;

use super::{Generator, rust_ident};

impl Generator<'_> {
    pub(super) fn provider<'a>(
        interface: &str,
        functions: impl Iterator<Item = &'a Function>,
    ) -> syn::Result<TokenStream> {
        let arms = functions
            .map(Self::provider_arm)
            .collect::<syn::Result<Vec<_>>>()?;
        Ok(quote! {
            #[doc = concat!("Wraps a `", #interface, "` host for registration with an app.")]
            #[must_use]
            pub fn provider(host: impl Host) -> ::wasm_junction::Provided {
                ::wasm_junction::Provided::new(INTERFACE, HostProvider(host))
            }

            struct HostProvider<T>(T);

            impl<T: Host> ::wasm_junction::Provider for HostProvider<T> {
                fn call<'a>(
                    &'a self,
                    __wasm_junction_cx: &'a ::wasm_junction::CallContext,
                    __wasm_junction_call: ::wasm_junction::Call,
                ) -> ::wasm_junction::BoxFuture<
                    'a,
                    ::std::result::Result<::wasm_junction::Vals, ::wasm_junction::CallError>,
                > {
                    ::std::boxed::Box::pin(async move {
                        match __wasm_junction_call.function.as_ref() {
                            #(#arms,)*
                            function => Err(::wasm_junction::CallError::refused(
                                ::std::format!(
                                    "unknown function `{function}` for `{}`",
                                    INTERFACE,
                                )
                            )),
                        }
                    })
                }
            }
        })
    }

    fn provider_arm(function: &Function) -> syn::Result<TokenStream> {
        let wit_name = &function.name;
        let method = rust_ident(&wit_name.to_snake_case())?;
        let call = rust_ident(&wit_name.to_upper_camel_case())?;
        let parameters = function
            .params
            .iter()
            .map(|param| rust_ident(&param.name.to_snake_case()))
            .collect::<syn::Result<Vec<_>>>()?;
        let invoke = quote!(<T as Host>::#method(
            &self.0,
            __wasm_junction_cx,
            #(#parameters),*
        ));
        let invoke = if function.kind.is_async() {
            quote!(#invoke.await)
        } else {
            invoke
        };
        Ok(quote! {
            #wit_name => {
                let #call { #(#parameters,)* } =
                    <#call as ::wasm_junction::TypedCall>::from_vals(
                        &__wasm_junction_call.args,
                    )?;
                Ok(<#call as ::wasm_junction::TypedCall>::output(#invoke))
            }
        })
    }
}
