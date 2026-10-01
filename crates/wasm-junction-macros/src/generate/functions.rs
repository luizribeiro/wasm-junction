use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::Function;

use super::Generator;
use super::collisions::{call_ident, parameter_ident};

impl Generator<'_> {
    pub(super) fn typed_call(
        &self,
        interface: &str,
        function: &Function,
    ) -> syn::Result<TokenStream> {
        let call = call_ident(interface, &function.name)?;
        let function_name = &function.name;
        let fields = function
            .params
            .iter()
            .map(|param| parameter_ident(&param.name))
            .collect::<syn::Result<Vec<_>>>()?;
        let field_types = function
            .params
            .iter()
            .map(|param| self.rust_type(param.ty, function_name))
            .collect::<syn::Result<Vec<_>>>()?;
        let decoded = function
            .params
            .iter()
            .zip(&fields)
            .map(|(param, field)| self.decode(param.ty, quote!(#field.clone()), function_name))
            .collect::<syn::Result<Vec<_>>>()?;
        let encoded = function
            .params
            .iter()
            .zip(&fields)
            .map(|(param, field)| self.encode(param.ty, quote!(#field), function_name))
            .collect::<syn::Result<Vec<_>>>()?;
        let output = function
            .result
            .map_or_else(|| Ok(quote!(())), |ty| self.rust_type(ty, function_name))?;
        let argument_error = format!(
            "{interface}.{function_name} expected {} arguments",
            fields.len()
        );
        let (encode_output, decode_output) = if let Some(ty) = function.result {
            let encode = self.encode(ty, quote!(value), function_name)?;
            let decode = self.decode(ty, quote!(value.clone()), function_name)?;
            let error = format!("{interface}.{function_name} expected one result");
            (
                quote!(::std::vec![#encode]),
                quote! {
                    let [value] = values else {
                        return Err(::wasm_junction::TypeError::new(#error));
                    };
                    #decode
                },
            )
        } else {
            let error = format!("{interface}.{function_name} expected no results");
            (
                quote!({
                    let _ = value;
                    ::std::vec::Vec::new()
                }),
                quote! {
                    if values.is_empty() { Ok(()) } else {
                        Err(::wasm_junction::TypeError::new(#error))
                    }
                },
            )
        };
        Ok(quote! {
            #[doc = concat!("Typed view of a `", #interface, ".", #function_name, "` call.")]
            pub struct #call {
                #(#[doc = concat!("The WIT `", stringify!(#fields), "` argument.")]
                  pub #fields: #field_types,)*
            }

            impl ::wasm_junction::TypedCall for #call {
                type Output = #output;
                const INTERFACE: &'static str = INTERFACE;
                const FUNCTION: &'static str = #function_name;

                fn from_vals(values: &[::wasm_junction::Val]) ->
                    ::std::result::Result<Self, ::wasm_junction::TypeError>
                {
                    let [#(#fields,)*] = values else {
                        return Err(::wasm_junction::TypeError::new(#argument_error));
                    };
                    Ok(Self { #(#fields: #decoded?,)* })
                }

                fn into_vals(self) -> ::wasm_junction::Vals {
                    let Self { #(#fields,)* } = self;
                    ::std::vec![#(#encoded,)*]
                }

                fn output(value: Self::Output) -> ::wasm_junction::Vals { #encode_output }

                fn decode_output(values: &[::wasm_junction::Val]) ->
                    ::std::result::Result<Self::Output, ::wasm_junction::TypeError>
                { #decode_output }
            }
        })
    }
}
