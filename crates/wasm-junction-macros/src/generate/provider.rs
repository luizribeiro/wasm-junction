use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Function, Handle, InterfaceId, Type, TypeDefKind};

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
        let arguments = function
            .params
            .iter()
            .zip(&parameters)
            .map(|(param, name)| self.provider_argument(param.ty, name))
            .collect::<syn::Result<Vec<_>>>()?;
        let preparations = arguments.iter().map(|(preparation, _)| preparation);
        let arguments = arguments.iter().map(|(_, argument)| argument);
        let invoke = quote!(<T as Host>::#method(
            &self.host,
            __wasm_junction_cx,
            #(#arguments),*
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
                #(#preparations)*
                Ok(<#call as ::wasm_junction::TypedCall>::output(#invoke))
            }
        })
    }

    fn provider_argument(
        &self,
        ty: Type,
        name: &proc_macro2::Ident,
    ) -> syn::Result<(TokenStream, TokenStream)> {
        let Type::Id(id) = ty else {
            return Ok((quote!(), quote!(#name)));
        };
        match &self.resolve.types[id].kind {
            TypeDefKind::Type(ty) => self.provider_argument(*ty, name),
            TypeDefKind::Handle(_) => self.map_handle(id, name),
            TypeDefKind::Option(ty) => {
                let Some(resource) = self.direct_resource(*ty)? else {
                    return Ok((quote!(), quote!(#name)));
                };
                let table = resource.table;
                if resource.borrowed {
                    let storage = super::rust_ident(&format!("__wasm_junction_{name}_borrow"))?;
                    Ok((
                        quote!(let #storage = #name.as_ref()
                            .map(|value| self.#table.borrow(value)).transpose()?;),
                        quote!(#storage.as_deref()),
                    ))
                } else {
                    Ok((
                        quote!(),
                        quote!(#name.map(|value| self.#table.take(&value)).transpose()?),
                    ))
                }
            }
            TypeDefKind::Result(result) => self.map_result(result.ok, result.err, name),
            _ => Ok((quote!(), quote!(#name))),
        }
    }

    fn map_handle(
        &self,
        id: wit_parser::TypeId,
        name: &proc_macro2::Ident,
    ) -> syn::Result<(TokenStream, TokenStream)> {
        let resource = self
            .direct_resource(Type::Id(id))?
            .ok_or_else(|| Self::unsupported(&name.to_string(), "resource handle"))?;
        let table = resource.table;
        if resource.borrowed {
            let storage = super::rust_ident(&format!("__wasm_junction_{name}_borrow"))?;
            Ok((
                quote!(let #storage = self.#table.borrow(&#name)?;),
                quote!(#storage.as_ref()),
            ))
        } else {
            Ok((quote!(), quote!(self.#table.take(&#name)?)))
        }
    }

    fn direct_resource(&self, ty: Type) -> syn::Result<Option<ResourceUse>> {
        let Type::Id(id) = ty else { return Ok(None) };
        match &self.resolve.types[id].kind {
            TypeDefKind::Type(ty) => self.direct_resource(*ty),
            TypeDefKind::Handle(handle) => {
                let (id, borrowed) = match handle {
                    Handle::Own(id) => (*id, false),
                    Handle::Borrow(id) => (*id, true),
                };
                let name = self.resolve.types[id]
                    .name
                    .as_deref()
                    .ok_or_else(|| Self::unsupported("resource", "anonymous resource"))?;
                Ok(Some(ResourceUse {
                    table: super::rust_ident(&name.to_snake_case())?,
                    borrowed,
                }))
            }
            _ => Ok(None),
        }
    }

    fn map_result(
        &self,
        ok: Option<Type>,
        err: Option<Type>,
        name: &proc_macro2::Ident,
    ) -> syn::Result<(TokenStream, TokenStream)> {
        let ok = ok.map(|ty| self.direct_resource(ty)).transpose()?.flatten();
        let err = err
            .map(|ty| self.direct_resource(ty))
            .transpose()?
            .flatten();
        let ok_store = Self::borrowed_result(ok.as_ref(), name, "ok")?;
        let err_store = Self::borrowed_result(err.as_ref(), name, "err")?;
        let preparations = [&ok_store, &err_store]
            .into_iter()
            .filter_map(|value| value.as_ref().map(|(_, tokens)| tokens));
        let ok_arm = Self::result_arm(ok.as_ref(), ok_store.as_ref(), true)?;
        let err_arm = Self::result_arm(err.as_ref(), err_store.as_ref(), false)?;
        Ok((
            quote!(#(#preparations)*),
            quote!(match #name { #ok_arm, #err_arm }),
        ))
    }

    fn borrowed_result(
        resource: Option<&ResourceUse>,
        name: &proc_macro2::Ident,
        side: &str,
    ) -> syn::Result<Option<(proc_macro2::Ident, TokenStream)>> {
        let Some(resource) = resource.filter(|resource| resource.borrowed) else {
            return Ok(None);
        };
        let storage = super::rust_ident(&format!("__wasm_junction_{name}_{side}_borrow"))?;
        let table = &resource.table;
        let pattern = if side == "ok" {
            quote!(Ok)
        } else {
            quote!(Err)
        };
        Ok(Some((
            storage.clone(),
            quote! {
                let #storage = match &#name {
                    #pattern(value) => Some(self.#table.borrow(value)?),
                    _ => None,
                };
            },
        )))
    }

    fn result_arm(
        resource: Option<&ResourceUse>,
        storage: Option<&(proc_macro2::Ident, TokenStream)>,
        ok: bool,
    ) -> syn::Result<TokenStream> {
        let constructor = if ok { quote!(Ok) } else { quote!(Err) };
        let Some(resource) = resource else {
            return Ok(quote!(#constructor(value) => #constructor(value)));
        };
        if resource.borrowed {
            let (storage, _) = storage
                .ok_or_else(|| Self::unsupported("result", "missing resource borrow storage"))?;
            Ok(quote! {
                #constructor(_) => {
                    let Some(value) = #storage.as_deref() else {
                        return Err(::wasm_junction::CallError::trap(
                            "resource result borrow was not prepared",
                        ));
                    };
                    #constructor(value)
                }
            })
        } else {
            let table = &resource.table;
            Ok(quote!(#constructor(value) => #constructor(self.#table.take(&value)?)))
        }
    }
}

struct ResourceUse {
    table: proc_macro2::Ident,
    borrowed: bool,
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
        assert!(tokens.contains("self . session . borrow"), "{tokens}");
        assert!(tokens.contains("self . session . take"), "{tokens}");
        assert!(tokens.contains("as_deref"), "{tokens}");
        assert!(tokens.contains("value_ok_borrow"), "{tokens}");
        assert!(tokens.contains("match value"), "{tokens}");
    }
}
