//! Procedural macros for generating wasm-junction bindings.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;

use proc_macro::TokenStream;
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{LitStr, Token, braced};
use wit_parser::Resolve;

mod generate;

struct Config {
    path: LitStr,
}

impl Parse for Config {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let content;
        braced!(content in input);
        let key: Ident = content.parse()?;
        if key != "path" {
            return Err(syn::Error::new(key.span(), "expected `path`"));
        }
        content.parse::<Token![:]>()?;
        let path = content.parse()?;
        if !content.is_empty() {
            content.parse::<Token![,]>()?;
        }
        if !content.is_empty() {
            return Err(content.error("unexpected bindgen option"));
        }
        Ok(Self { path })
    }
}

/// Generates bindings for every interface in a local WIT package.
#[proc_macro]
pub fn bindgen(input: TokenStream) -> TokenStream {
    syn::parse::<Config>(input)
        .and_then(|config| expand(&config))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(config: &Config) -> syn::Result<TokenStream2> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| syn::Error::new(config.path.span(), "CARGO_MANIFEST_DIR is not set"))?;
    let path = PathBuf::from(manifest).join(config.path.value());
    let mut resolve = Resolve::default();
    let (package_id, sources) = resolve
        .push_path(&path)
        .map_err(|error| syn::Error::new(config.path.span(), format!("{error:#}")))?;
    let tracked = sources.paths().map(|path| {
        let path = path.to_string_lossy();
        quote!(
            const _: &[u8] = include_bytes!(#path);
        )
    });
    let bindings = generate::generate(&resolve, package_id, config.path.span())?;
    Ok(quote!(#(#tracked)* #bindings))
}

#[cfg(test)]
mod tests {
    use proc_macro2::Span;
    use syn::LitStr;

    use super::{Config, expand};

    #[test]
    fn bad_path_value_reports_the_expected_syntax() {
        let Err(error) = syn::parse_str::<Config>("{ path: 42 }") else {
            panic!("non-string path was accepted");
        };
        assert!(error.to_string().contains("expected string literal"));
    }

    #[test]
    fn missing_path_reports_the_directory_and_io_cause() {
        let config = Config {
            path: LitStr::new("tests/fixtures/does-not-exist", Span::call_site()),
        };
        let error = expand(&config).unwrap_err().to_string();
        assert!(error.contains("does-not-exist"));
        assert!(error.contains("No such file or directory"));
    }

    #[test]
    fn wit_syntax_error_keeps_the_parser_cause() {
        let config = Config {
            path: LitStr::new("tests/fixtures/malformed", Span::call_site()),
        };
        let error = expand(&config).unwrap_err().to_string();
        assert!(error.contains("malformed"), "{error}");
        assert!(error.contains("expected an identifier"), "{error}");
    }
}
