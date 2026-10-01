use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Function, Type, TypeDefKind};

use super::Generator;
use super::collisions::{call_ident, handle_ident, method_ident, parameter_ident};

impl Generator<'_> {
    pub(super) fn handle<'a>(
        &self,
        interface: &str,
        functions: impl Iterator<Item = &'a Function>,
    ) -> syn::Result<TokenStream> {
        let handle = handle_ident(interface)?;
        let methods = functions
            .map(|function| self.handle_method(interface, function))
            .collect::<syn::Result<Vec<_>>>()?;
        Ok(quote! {
            #[doc = concat!("A typed handle for a component's `", #interface, "` export.")]
            #[derive(Clone)]
            pub struct #handle {
                handle: ::wasm_junction::Handle,
            }

            impl #handle {
                /// Returns a copy whose calls carry `value` as per-call data.
                #[must_use]
                pub fn with<T: ::std::any::Any + ::wasm_junction::HostBound>(
                    &self,
                    value: T,
                ) -> Self {
                    Self { handle: self.handle.with(value) }
                }

                /// Returns a copy whose calls continue the invocation in `context`.
                #[must_use]
                pub fn within(&self, context: &::wasm_junction::CallContext) -> Self {
                    Self { handle: self.handle.within(context) }
                }

                #(#methods)*
            }

            impl ::wasm_junction::InterfaceHandle for #handle {
                const INTERFACE: &'static str = INTERFACE;

                fn from_app(
                    app: ::wasm_junction::App,
                    component: ::std::sync::Arc<str>,
                ) -> Self {
                    Self { handle: ::wasm_junction::Handle::new(app, component) }
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
        let (output, convert_output) = function.result.map_or_else(
            || Ok((quote!(()), quote!(Ok(value)))),
            |ty| self.handle_output(ty, &quote!(value), &function.name),
        )?;
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
                let values = self.handle.call(
                    INTERFACE,
                    <#call as ::wasm_junction::TypedCall>::FUNCTION,
                    <#call as ::wasm_junction::TypedCall>::into_vals(
                        #call { #(#names: #values,)* }
                    ),
                ).await?;
                let value = <#call as ::wasm_junction::TypedCall>::decode_output(&values)
                    .map_err(::wasm_junction::CallError::from)?;
                #convert_output
            }
        })
    }

    fn handle_argument(
        &self,
        ty: Type,
        item: &str,
        value: &proc_macro2::Ident,
    ) -> syn::Result<(TokenStream, TokenStream)> {
        if let Some(mapped) =
            self.handle_stream(ty, &quote!(#value), item, StreamDirection::Output)?
        {
            return Ok(mapped);
        }
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

    fn handle_output(
        &self,
        ty: Type,
        value: &TokenStream,
        item: &str,
    ) -> syn::Result<(TokenStream, TokenStream)> {
        self.handle_stream(ty, value, item, StreamDirection::Input)?
            .map_or_else(|| Ok((self.rust_type(ty, item)?, quote!(Ok(value)))), Ok)
    }

    fn handle_stream(
        &self,
        ty: Type,
        value: &TokenStream,
        item: &str,
        direction: StreamDirection,
    ) -> syn::Result<Option<(TokenStream, TokenStream)>> {
        if self.direct_stream(ty) {
            return Ok(Some((direction.ty(), direction.convert(value))));
        }
        let Type::Id(id) = ty else { return Ok(None) };
        match &self.resolve.types[id].kind {
            TypeDefKind::Option(payload) if self.direct_stream(*payload) => {
                let ty = direction.ty();
                let mapped = match direction {
                    StreamDirection::Output => {
                        quote!(#value.map(::std::convert::Into::into))
                    }
                    StreamDirection::Input => quote!(#value
                        .map(::wasm_junction::InputStream::try_from)
                        .transpose()
                        .map_err(|error|
                            ::wasm_junction::CallError::trap(error.to_string()))),
                };
                Ok(Some((quote!(::std::option::Option<#ty>), mapped)))
            }
            TypeDefKind::Result(result)
                if result.ok.is_some_and(|ty| self.direct_stream(ty))
                    || result.err.is_some_and(|ty| self.direct_stream(ty)) =>
            {
                let (ok_ty, ok) = self.handle_result_side(result.ok, true, item, direction)?;
                let (err_ty, err) = self.handle_result_side(result.err, false, item, direction)?;
                Ok(Some((
                    quote!(::std::result::Result<#ok_ty, #err_ty>),
                    quote!(match #value { #ok, #err }),
                )))
            }
            _ => Ok(None),
        }
    }

    fn handle_result_side(
        &self,
        ty: Option<Type>,
        ok: bool,
        item: &str,
        direction: StreamDirection,
    ) -> syn::Result<(TokenStream, TokenStream)> {
        let constructor = if ok { quote!(Ok) } else { quote!(Err) };
        let Some(ty) = ty else {
            return Ok((quote!(()), quote!(#constructor(()) => #constructor(()))));
        };
        if self.direct_stream(ty) {
            let mapped = direction.convert(&quote!(value));
            let arm = match direction {
                StreamDirection::Output => {
                    quote!(#constructor(value) => #constructor(#mapped))
                }
                StreamDirection::Input => {
                    quote!(#constructor(value) => #mapped.map(#constructor))
                }
            };
            Ok((direction.ty(), arm))
        } else {
            Ok((
                self.rust_type(ty, item)?,
                quote!(#constructor(value) => #constructor(value)),
            ))
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

#[derive(Clone, Copy)]
enum StreamDirection {
    Input,
    Output,
}

impl StreamDirection {
    fn ty(self) -> TokenStream {
        match self {
            Self::Output => quote!(::wasm_junction::OutputStream),
            Self::Input => quote!(::wasm_junction::InputStream),
        }
    }

    fn convert(self, value: &TokenStream) -> TokenStream {
        match self {
            Self::Output => quote!(::std::convert::Into::<
                ::wasm_junction::StreamHandle,
            >::into(#value)),
            Self::Input => quote!(::wasm_junction::InputStream::try_from(#value)
                .map_err(|error| ::wasm_junction::CallError::trap(error.to_string()))),
        }
    }
}
