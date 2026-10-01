use heck::ToUpperCamelCase;
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Function, Type, TypeDefKind};

use super::Generator;
use super::collisions::{call_ident, method_ident, parameter_ident};
use super::rust_ident;

impl Generator<'_> {
    pub(super) fn handle<'a>(
        &self,
        interface: &str,
        functions: impl Iterator<Item = &'a Function>,
    ) -> syn::Result<TokenStream> {
        let handle = rust_ident(&interface.to_upper_camel_case())?;
        let methods = functions
            .map(|function| self.handle_method(interface, function))
            .collect::<syn::Result<Vec<_>>>()?;
        Ok(quote! {
            #[doc = concat!("A typed handle for a component's `", #interface, "` export.")]
            #[derive(Clone)]
            pub struct #handle {
                app: ::wasm_junction::App,
                component: ::std::sync::Arc<str>,
            }

            impl #handle { #(#methods)* }

            impl ::wasm_junction::InterfaceHandle for #handle {
                const INTERFACE: &'static str = INTERFACE;

                fn from_app(
                    app: ::wasm_junction::App,
                    component: ::std::sync::Arc<str>,
                ) -> Self {
                    Self { app, component }
                }
            }
        })
    }

    fn handle_method(&self, interface: &str, function: &Function) -> syn::Result<TokenStream> {
        let method = method_ident(&function.name)?;
        let call = call_ident(interface, &function.name)?;
        let names = function
            .params
            .iter()
            .map(|param| parameter_ident(&param.name))
            .collect::<syn::Result<Vec<_>>>()?;
        let arguments = function
            .params
            .iter()
            .zip(&names)
            .map(|(param, name)| self.handle_argument(param.ty, &function.name, name))
            .collect::<syn::Result<Vec<_>>>()?;
        let (types, values): (Vec<_>, Vec<_>) = arguments.into_iter().unzip();
        let output = function
            .result
            .map_or_else(|| Ok(quote!(())), |ty| self.rust_type(ty, &function.name))?;
        let function_name = &function.name;
        Ok(quote! {
            #[doc = concat!("Calls the WIT `", #function_name, "` function.")]
            ///
            /// # Errors
            ///
            /// Returns a [`::wasm_junction::CallError`] if the call cannot complete.
            pub async fn #method(&self, #(#names: #types),*) ->
                ::std::result::Result<#output, ::wasm_junction::CallError>
            {
                let values = self.app.call(
                    &self.component,
                    INTERFACE,
                    <#call as ::wasm_junction::TypedCall>::FUNCTION,
                    <#call as ::wasm_junction::TypedCall>::into_vals(
                        #call { #(#names: #values,)* }
                    ),
                ).await?;
                <#call as ::wasm_junction::TypedCall>::decode_output(&values)
                    .map_err(::wasm_junction::CallError::from)
            }
        })
    }

    fn handle_argument(
        &self,
        ty: Type,
        item: &str,
        value: &proc_macro2::Ident,
    ) -> syn::Result<(TokenStream, TokenStream)> {
        let owned = self.rust_type(ty, item)?;
        match self.argument_kind(ty) {
            ArgumentKind::Value => Ok((owned, quote!(#value))),
            ArgumentKind::String => Ok((quote!(&str), quote!(#value.to_owned()))),
            ArgumentKind::List(element) => {
                let element = self.rust_type(element, item)?;
                Ok((quote!(&[#element]), quote!(#value.to_vec())))
            }
            ArgumentKind::StringList => Ok((
                quote!(&[impl ::std::convert::AsRef<str>]),
                quote!(#value.iter().map(|value| value.as_ref().to_owned()).collect()),
            )),
            ArgumentKind::Option(element) => {
                let element_type = self.rust_type(element, item)?;
                match self.argument_kind(element) {
                    ArgumentKind::String => Ok((
                        quote!(::std::option::Option<&str>),
                        quote!(#value.map(str::to_owned)),
                    )),
                    ArgumentKind::Value => {
                        Ok((quote!(::std::option::Option<#element_type>), quote!(#value)))
                    }
                    _ => Ok((
                        quote!(::std::option::Option<&#element_type>),
                        quote!(#value.cloned()),
                    )),
                }
            }
            ArgumentKind::Borrowed => Ok((quote!(&#owned), quote!(#value.clone()))),
        }
    }

    fn argument_kind(&self, ty: Type) -> ArgumentKind {
        let Type::Id(id) = ty else {
            return if ty == Type::String {
                ArgumentKind::String
            } else {
                ArgumentKind::Value
            };
        };
        match &self.resolve.types[id].kind {
            TypeDefKind::Type(target) => self.argument_kind(*target),
            TypeDefKind::List(element)
                if matches!(self.argument_kind(*element), ArgumentKind::String) =>
            {
                ArgumentKind::StringList
            }
            TypeDefKind::List(element) => ArgumentKind::List(*element),
            TypeDefKind::Option(element) => ArgumentKind::Option(*element),
            TypeDefKind::Enum(_) | TypeDefKind::Flags(_) => ArgumentKind::Value,
            _ => ArgumentKind::Borrowed,
        }
    }
}

enum ArgumentKind {
    Value,
    String,
    List(Type),
    StringList,
    Option(Type),
    Borrowed,
}
