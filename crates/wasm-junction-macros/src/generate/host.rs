use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Function, Handle, InterfaceId, Type, TypeDefKind};

use super::Generator;
use super::collisions::{host_method_ident, host_parameter_idents, resource_ident};

impl Generator<'_> {
    pub(super) fn host_trait<'a>(
        &self,
        interface: &str,
        interface_id: InterfaceId,
        functions: impl Iterator<Item = &'a Function> + Clone,
    ) -> syn::Result<TokenStream> {
        let resources = self.resources(interface, interface_id)?;
        let associated = resources.iter().map(|resource| {
            let name = resource.name;
            let ident = &resource.ident;
            quote! {
                #[doc = concat!("The host value stored for the WIT `", #name, "` resource.")]
                type #ident: ::wasm_junction::HostBound;
            }
        });
        let forwarding_types = resources.iter().map(|resource| {
            let ident = &resource.ident;
            quote!(type #ident = T::#ident;)
        });
        let drops = resources
            .iter()
            .map(|resource| {
                let name = resource.name;
                let ident = &resource.ident;
                let method = super::rust_ident(&format!("drop_{}", name.to_snake_case()))?;
                Ok(quote! {
                    #[doc = concat!("Drops the host value for a released `", #name, "` handle.")]
                    fn #method(&self, _cx: &::wasm_junction::CallContext, value: Self::#ident) {
                        ::std::mem::drop(value);
                    }
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let forwarding_drops = resources
            .iter()
            .map(|resource| {
                let name = resource.name;
                let ident = &resource.ident;
                let method = super::rust_ident(&format!("drop_{}", name.to_snake_case()))?;
                Ok(quote! {
                    fn #method(&self, cx: &::wasm_junction::CallContext, value: Self::#ident) {
                        self.as_ref().#method(cx, value);
                    }
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let methods = functions
            .clone()
            .map(|function| self.host_method(interface, function, false))
            .collect::<syn::Result<Vec<_>>>()?;
        let forwarding = functions
            .map(|function| self.host_method(interface, function, true))
            .collect::<syn::Result<Vec<_>>>()?;
        Ok(quote! {
            #[doc = concat!("A host implementation of the `", #interface, "` interface.")]
            pub trait Host: ::wasm_junction::HostBound + Sized + 'static {
                #(#associated)*
                #(#methods)*
                #(#drops)*
            }

            impl<T: Host> Host for ::std::sync::Arc<T> {
                #(#forwarding_types)*
                #(#forwarding)*
                #(#forwarding_drops)*
            }
        })
    }

    fn host_method(
        &self,
        interface: &str,
        function: &Function,
        forward: bool,
    ) -> syn::Result<TokenStream> {
        let wit_name = &function.name;
        let resource = function
            .kind
            .resource()
            .and_then(|id| self.resolve.types[id].name.as_deref());
        let method = host_method_ident(function, resource)?;
        let parameters = host_parameter_idents(function, resource)?;
        let parameter_types = function
            .params
            .iter()
            .map(|param| self.host_type(interface, param.ty, wit_name))
            .collect::<syn::Result<Vec<_>>>()?;
        let output = function.result.map_or_else(
            || Ok(quote!(())),
            |ty| self.host_output_type(interface, ty, wit_name),
        )?;
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

    fn host_type(&self, interface: &str, ty: Type, item: &str) -> syn::Result<TokenStream> {
        let Type::Id(id) = ty else {
            return self.rust_type(ty, item);
        };
        match &self.resolve.types[id].kind {
            TypeDefKind::Handle(handle) => {
                let (resource, borrowed) = match handle {
                    Handle::Own(id) => (*id, false),
                    Handle::Borrow(id) => (*id, true),
                };
                let name = self.resolve.types[resource]
                    .name
                    .as_deref()
                    .ok_or_else(|| Self::unsupported(item, "anonymous resource"))?;
                let ident = resource_ident(interface, name)?;
                Ok(if borrowed {
                    quote!(&Self::#ident)
                } else {
                    quote!(Self::#ident)
                })
            }
            TypeDefKind::Type(ty) => self.host_type(interface, *ty, item),
            TypeDefKind::Stream(Some(Type::U8)) => Ok(quote!(::wasm_junction::InputStream)),
            TypeDefKind::Option(ty) => {
                let ty = self.host_type(interface, *ty, item)?;
                Ok(quote!(::std::option::Option<#ty>))
            }
            TypeDefKind::Result(result) => {
                let ok = result
                    .ok
                    .map_or_else(|| Ok(quote!(())), |ty| self.host_type(interface, ty, item))?;
                let err = result
                    .err
                    .map_or_else(|| Ok(quote!(())), |ty| self.host_type(interface, ty, item))?;
                Ok(quote!(::std::result::Result<#ok, #err>))
            }
            _ => self.rust_type(ty, item),
        }
    }

    fn host_output_type(&self, interface: &str, ty: Type, item: &str) -> syn::Result<TokenStream> {
        let Type::Id(id) = ty else {
            return self.rust_type(ty, item);
        };
        match &self.resolve.types[id].kind {
            TypeDefKind::Type(ty) => self.host_output_type(interface, *ty, item),
            TypeDefKind::Stream(Some(Type::U8)) => Ok(quote!(::wasm_junction::OutputStream)),
            TypeDefKind::Option(ty) => {
                let ty = self.host_output_type(interface, *ty, item)?;
                Ok(quote!(::std::option::Option<#ty>))
            }
            TypeDefKind::Result(result) => {
                let ok = result.ok.map_or_else(
                    || Ok(quote!(())),
                    |ty| self.host_output_type(interface, ty, item),
                )?;
                let err = result.err.map_or_else(
                    || Ok(quote!(())),
                    |ty| self.host_output_type(interface, ty, item),
                )?;
                Ok(quote!(::std::result::Result<#ok, #err>))
            }
            _ => self.host_type(interface, ty, item),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;

    use wit_parser::Resolve;

    use super::Generator;

    #[test]
    fn resources_generate_host_values_and_drop_hooks() {
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
            .host_trait(
                "host",
                interface,
                resolve.interfaces[interface].functions.values(),
            )
            .unwrap()
            .to_string();

        assert!(tokens.contains("type Session"), "{tokens}");
        assert!(tokens.contains("fn drop_session"), "{tokens}");
        assert!(tokens.contains("Option < & Self :: Session >"), "{tokens}");
        assert!(tokens.contains("Result < Self :: Session"), "{tokens}");
        assert!(tokens.contains("type Session = T :: Session"), "{tokens}");
    }
}
