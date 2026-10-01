use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Function, InterfaceId};

use super::Generator;
use super::collisions::{call_ident, host_method_ident, host_parameter_idents, parameter_ident};

impl Generator<'_> {
    pub(super) fn provider<'a>(
        &self,
        interface: &str,
        interface_id: InterfaceId,
        functions: impl Iterator<Item = &'a Function>,
    ) -> syn::Result<TokenStream> {
        let resources = self
            .resources(interface, interface_id)?
            .into_iter()
            .map(|resource| {
                Ok((
                    resource.name,
                    super::rust_ident(&resource.name.to_snake_case())?,
                    resource.ident,
                ))
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let arms = functions
            .map(|function| self.provider_arm(interface, function))
            .collect::<syn::Result<Vec<_>>>()?;
        let fields = resources.iter().map(
            |(_, field, associated)| quote!(#field: ::wasm_junction::ResourceTable<T::#associated>),
        );
        let tables = resources.iter().map(|(name, field, _)| {
            quote!(#field: ::wasm_junction::ResourceTable::new(INTERFACE, #name))
        });
        let drops = resources
            .iter()
            .map(|(name, field, associated)| {
                let method = super::rust_ident(&format!("drop_{field}"))?;
                Ok(quote! {
                    #name => {
                        let value: T::#associated = self.#field.take(&resource)?;
                        <T as Host>::#method(&self.host, cx, value);
                        Ok(())
                    }
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let drop_impl = (!resources.is_empty()).then(|| {
            quote! {
                fn drop_resource(
                    &self,
                    cx: &::wasm_junction::CallContext,
                    resource: ::wasm_junction::Resource,
                ) -> ::std::result::Result<(), ::wasm_junction::CallError> {
                    match resource.name() {
                        #(#drops,)*
                        name => Err(::wasm_junction::CallError::unavailable(
                            ::std::format!("unknown resource `{name}` for `{}`", INTERFACE),
                        )),
                    }
                }
            }
        });
        Ok(quote! {
            #[doc = concat!("Wraps a `", #interface, "` host for registration with an app.")]
            #[must_use]
            pub fn provider(host: impl Host) -> ::wasm_junction::Provided {
                ::wasm_junction::Provided::new(INTERFACE, HostProvider {
                    host,
                    #(#tables,)*
                })
            }

            struct HostProvider<T: Host> {
                host: T,
                #(#fields,)*
            }

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

                #drop_impl
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
            &self.host,
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

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;

    use wit_parser::Resolve;

    use super::Generator;

    #[test]
    fn providers_store_and_drop_each_resource_type() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resource-nesting");
        let mut resolve = Resolve::default();
        let (package, _) = resolve.push_path(path).unwrap();
        let interface = resolve.packages[package].interfaces["host"];
        let generator = Generator {
            resolve: &resolve,
            errors: HashSet::default(),
            selected: HashMap::default(),
            with: HashMap::default(),
        };
        let tokens = generator
            .provider(
                "host",
                interface,
                resolve.interfaces[interface].functions.values(),
            )
            .unwrap()
            .to_string();

        assert!(
            tokens.contains("ResourceTable < T :: Session >"),
            "{tokens}"
        );
        assert!(
            tokens.contains("ResourceTable :: new (INTERFACE , \"session\")"),
            "{tokens}"
        );
        assert!(tokens.contains("fn drop_resource"), "{tokens}");
        assert!(tokens.contains("Host > :: drop_session"), "{tokens}");
    }
}
