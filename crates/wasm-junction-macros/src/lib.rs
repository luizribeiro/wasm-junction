//! Procedural macros for generating wasm-junction bindings.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;

use proc_macro::TokenStream;
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{LitStr, Path, Token, braced, bracketed};
use wit_parser::Resolve;

mod generate;

struct Config {
    path: LitStr,
    interfaces: Option<Vec<LitStr>>,
    with: Vec<(LitStr, Path)>,
}

impl Parse for Config {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let content;
        braced!(content in input);
        let mut path = None;
        let mut interfaces = None;
        let mut with = None;
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
                "with" if with.is_none() => {
                    let mappings;
                    braced!(mappings in content);
                    let mut values = Vec::new();
                    while !mappings.is_empty() {
                        values.push((mappings.parse()?, {
                            mappings.parse::<Token![:]>()?;
                            mappings.parse()?
                        }));
                        if !mappings.is_empty() {
                            mappings.parse::<Token![,]>()?;
                        }
                    }
                    with = Some(values);
                }
                "path" | "interfaces" | "with" => {
                    return Err(syn::Error::new(key.span(), "duplicate bindgen option"));
                }
                _ => return Err(syn::Error::new(key.span(), "unexpected bindgen option")),
            }
            if !content.is_empty() {
                content.parse::<Token![,]>()?;
            }
        }
        let path = path.ok_or_else(|| content.error("missing `path` option"))?;
        Ok(Self {
            path,
            interfaces,
            with: with.unwrap_or_default(),
        })
    }
}

/// Generates bindings for interfaces in a local WIT package.
///
/// The `interfaces` option narrows generation to fully qualified interface
/// names, with package versions optional. The `with` option maps a WIT package
/// to an existing bindings path, preserving one Rust type identity for the
/// entire reused package. Single-interface `with` keys are not supported
/// because they could split one package's type universe across invocations.
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
        &config.with,
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
            with: Vec::new(),
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
            with: Vec::new(),
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
            with: Vec::new(),
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
            with: Vec::new(),
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
            with: Vec::new(),
        };
        let tokens = expand(&config).unwrap().to_string();
        assert!(tokens.contains("mod notes"), "{tokens}");
        assert!(tokens.contains("mod types"), "{tokens}");
        assert!(!tokens.contains("mod unused"), "{tokens}");
    }

    #[test]
    fn with_reuses_a_package_path() {
        let config = syn::parse_str::<Config>(
            r#"{
                path: "../wasm-junction/tests/fixtures/dependencies/wit",
                with: { "test:common": crate::shared }
            }"#,
        )
        .unwrap();
        let tokens = expand(&config).unwrap().to_string();
        assert!(
            tokens.contains("crate :: shared :: types :: Author"),
            "{tokens}"
        );
        assert!(!tokens.contains("mod types"), "{tokens}");
    }

    #[test]
    fn with_rejects_interface_keys() {
        let config = syn::parse_str::<Config>(
            r#"{
                path: "../wasm-junction/tests/fixtures/dependencies/wit",
                with: { "test:common/types": crate::shared }
            }"#,
        )
        .unwrap();
        let error = expand(&config).unwrap_err().to_string();
        assert!(error.contains("keys must name WIT packages"));
    }

    #[test]
    fn with_requires_a_version_when_packages_are_ambiguous() {
        let config = syn::parse_str::<Config>(
            r#"{
                path: "tests/fixtures/versions/wit",
                with: { "test:shared": crate::shared }
            }"#,
        )
        .unwrap();
        let error = expand(&config).unwrap_err().to_string();
        assert!(error.contains("ambiguous WIT package `test:shared`"));
        assert!(error.contains("`test:shared@1.0.0`"));
        assert!(error.contains("`test:shared@2.0.0`"));
        assert!(error.contains("use `name@version`"));
    }

    #[test]
    fn with_accepts_a_qualified_ambiguous_package() {
        let config = syn::parse_str::<Config>(
            r#"{
                path: "tests/fixtures/versions/wit",
                with: { "test:shared@1.0.0": crate::shared }
            }"#,
        )
        .unwrap();
        let tokens = expand(&config).unwrap().to_string();
        assert!(
            tokens.contains("crate :: shared :: types :: Item"),
            "{tokens}"
        );
    }

    #[test]
    fn unknown_with_package_lists_available_packages() {
        let config = syn::parse_str::<Config>(
            r#"{
                path: "../wasm-junction/tests/fixtures/dependencies/wit",
                with: { "test:missing": crate::shared }
            }"#,
        )
        .unwrap();
        let error = expand(&config).unwrap_err().to_string();
        assert!(error.contains("unknown WIT package `test:missing`"));
        assert!(error.contains("`test:common`"));
        assert!(error.contains("`test:notes`"));
    }
}
