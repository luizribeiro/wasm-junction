use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::Function;

use super::Generator;
use super::collisions::{call_ident, host_method_ident, host_parameter_idents, parameter_ident};

impl Generator<'_> {
    pub(super) fn provider<'a>(
        &self,
        interface: &str,
        functions: impl Iterator<Item = &'a Function>,
    ) -> syn::Result<TokenStream> {
        let arms = functions
            .map(|function| self.provider_arm(interface, function))
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

    fn provider_arm(&self, interface: &str, function: &Function) -> syn::Result<TokenStream> {
        let wit_name = &function.name;
        let resource = function
            .kind
            .resource()
            .and_then(|id| self.resolve.types[id].name.as_deref());
        let method = host_method_ident(function, resource)?;
        let call = call_ident(interface, wit_name)?;
        let fields = function
            .params
            .iter()
            .map(|param| parameter_ident(&param.name))
            .collect::<syn::Result<Vec<_>>>()?;
        let parameters = host_parameter_idents(function, resource)?;
        let bindings = fields
            .iter()
            .zip(&parameters)
            .map(|(field, parameter)| quote!(#field: #parameter));
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
                let #call { #(#bindings,)* } =
                    <#call as ::wasm_junction::TypedCall>::from_vals(
                        &__wasm_junction_call.args,
                    )?;
                Ok(<#call as ::wasm_junction::TypedCall>::output(#invoke))
            }
        })
    }
}
