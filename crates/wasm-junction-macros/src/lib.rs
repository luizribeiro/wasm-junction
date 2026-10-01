//! Procedural macros for generating wasm-junction bindings.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;

use proc_macro::TokenStream;
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{LitStr, Token, braced, bracketed};
use wit_parser::Resolve;

mod generate;

struct Config {
    path: LitStr,
    interfaces: Option<Vec<LitStr>>,
}

impl Parse for Config {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let content;
        braced!(content in input);
        let mut path = None;
        let mut interfaces = None;
        while !content.is_empty() {
            let key: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            match key.to_string().as_str() {
                "path" if path.is_none() => path = Some(content.parse()?),
                "interfaces" if interfaces.is_none() => {
                    let values;
                    bracketed!(values in content);
                    interfaces = Some(
                        values
                            .parse_terminated(syn::parse::ParseBuffer::parse, Token![,])?
                            .into_iter()
                            .collect(),
                    );
                }
                "path" | "interfaces" => {
                    return Err(syn::Error::new(key.span(), "duplicate bindgen option"));
                }
                _ => return Err(syn::Error::new(key.span(), "unexpected bindgen option")),
            }
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }
        let path = path.ok_or_else(|| content.error("missing `path` option"))?;
        Ok(Self { path, interfaces })
    }
}

/// Generates bindings for every interface in a local WIT package.
///
/// Generated WIT errors display enum and payload-free variant cases by their
/// kebab-case names. Variant payloads follow the case name and use `Display`
/// when available, otherwise a compact debug representation. Record errors
/// display comma-separated `field: value` pairs with the same value rule.
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
    let bindings = generate::generate(
        &resolve,
        package_id,
        config.interfaces.as_deref(),
        config.path.span(),
    )?;
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
            interfaces: None,
        };
        let error = expand(&config).unwrap_err().to_string();
        assert!(error.contains("does-not-exist"));
        assert!(error.contains("No such file or directory"));
    }

    #[test]
    fn wit_syntax_error_keeps_the_parser_cause() {
        let config = Config {
            path: LitStr::new("tests/fixtures/malformed", Span::call_site()),
            interfaces: None,
        };
        let error = expand(&config).unwrap_err().to_string();
        assert!(error.contains("malformed"), "{error}");
        assert!(error.contains("expected an identifier"), "{error}");
    }

    #[test]
    fn interfaces_accepts_names_without_versions_and_narrows_output() {
        let config = Config {
            path: LitStr::new(
                "../wasm-junction/tests/fixtures/modules/wit",
                Span::call_site(),
            ),
            interfaces: Some(vec![LitStr::new("test:names/search", Span::call_site())]),
        };
        let tokens = expand(&config).unwrap().to_string();
        assert!(tokens.contains("mod search"), "{tokens}");
        assert!(!tokens.contains("mod note_store"), "{tokens}");
    }

    #[test]
    fn unknown_interface_lists_the_available_interfaces() {
        let config = Config {
            path: LitStr::new(
                "../wasm-junction/tests/fixtures/modules/wit",
                Span::call_site(),
            ),
            interfaces: Some(vec![LitStr::new("test:names/missing", Span::call_site())]),
        };
        let error = expand(&config).unwrap_err().to_string();
        assert!(error.contains("unknown interface `test:names/missing`"));
        assert!(error.contains("`test:names/note-store@1.2.3`"));
        assert!(error.contains("`test:names/search@1.2.3`"));
    }

    #[test]
    fn interfaces_includes_referenced_types_only() {
        let config = Config {
            path: LitStr::new(
                "../wasm-junction/tests/fixtures/dependencies/wit",
                Span::call_site(),
            ),
            interfaces: Some(vec![LitStr::new("test:notes/notes", Span::call_site())]),
        };
        let tokens = expand(&config).unwrap().to_string();
        assert!(tokens.contains("mod notes"), "{tokens}");
        assert!(tokens.contains("mod types"), "{tokens}");
        assert!(!tokens.contains("mod unused"), "{tokens}");
    }
}
