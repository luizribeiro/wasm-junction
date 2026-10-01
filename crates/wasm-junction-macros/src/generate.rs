use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{LitStr, Path};
use wit_parser::{InterfaceId, PackageId, Resolve};

mod collisions;
mod definitions;
mod display;
mod errors;
mod functions;
mod handles;
mod host;
mod provider;
mod reachability;
mod types;
mod validate;
mod values;
mod walk;

pub(crate) fn generate(
    resolve: &Resolve,
    package_id: PackageId,
    requested: Option<&[LitStr]>,
    with: &[(LitStr, Path)],
    span: Span,
) -> syn::Result<TokenStream> {
    let roots = select_interfaces(resolve, package_id, requested)?;
    let with = resolve_with(resolve, with)?;
    validate::interfaces(resolve, &roots, span)?;
    collisions::check(resolve, &roots, &with.keys().copied().collect(), span)?;
    let mut selected = reachability::find(resolve, &roots)?;
    selected.retain(|id, _| {
        resolve.interfaces[*id]
            .package
            .is_none_or(|package| !with.contains_key(&package))
    });
    let generator = Generator {
        resolve,
        errors: errors::find(resolve, &roots)?,
        selected,
        with,
    };
    let modules = resolve
        .interfaces
        .iter()
        .filter(|(id, _)| generator.selected.contains_key(id))
        .map(|(id, interface)| {
            let name = interface.name.as_deref().ok_or_else(|| {
                syn::Error::new(Span::call_site(), "selected interface has no name")
            })?;
            generator.interface(name, id)
        })
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(quote!(#(#modules)*))
}

fn resolve_with(
    resolve: &Resolve,
    mappings: &[(LitStr, Path)],
) -> syn::Result<std::collections::HashMap<PackageId, Path>> {
    let available = resolve
        .packages
        .iter()
        .map(|(id, package)| (package.name.to_string(), id))
        .collect::<Vec<_>>();
    let mut result = std::collections::HashMap::new();
    for (name, path) in mappings {
        let value = name.value();
        if value.contains('/') {
            return Err(syn::Error::new(
                name.span(),
                "`with` keys must name WIT packages; interface keys are not supported",
            ));
        }
        let package = match_available(name, &available, "WIT package")?.ok_or_else(|| {
            let names = available_names(&available);
            syn::Error::new(
                name.span(),
                format!("unknown WIT package `{value}`; available packages: {names}"),
            )
        })?;
        if result.insert(package, path.clone()).is_some() {
            return Err(syn::Error::new(name.span(), "duplicate `with` package"));
        }
    }
    Ok(result)
}

fn match_available<T: Copy>(
    requested: &LitStr,
    available: &[(String, T)],
    kind: &str,
) -> syn::Result<Option<T>> {
    let value = requested.value();
    let matches = available
        .iter()
        .filter(|(candidate, _)| {
            if value.contains('@') {
                candidate == &value
            } else {
                candidate.split('@').next() == Some(&value)
            }
        })
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        let candidates = matches
            .iter()
            .map(|(name, _)| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(syn::Error::new(
            requested.span(),
            format!("ambiguous {kind} `{value}`; candidates: {candidates}; use `name@version`"),
        ));
    }
    Ok(matches.first().map(|(_, id)| *id))
}

fn available_names<T>(available: &[(String, T)]) -> String {
    available
        .iter()
        .map(|(name, _)| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn select_interfaces(
    resolve: &Resolve,
    package_id: PackageId,
    requested: Option<&[LitStr]>,
) -> syn::Result<Vec<InterfaceId>> {
    let available = resolve.packages[package_id]
        .interfaces
        .iter()
        .map(|(name, id)| (resolve.packages[package_id].name.interface_id(name), *id))
        .collect::<Vec<_>>();
    let Some(requested) = requested else {
        return Ok(available.iter().map(|(_, id)| *id).collect());
    };
    requested
        .iter()
        .map(|name| {
            let value = name.value();
            match_available(name, &available, "interface")?.ok_or_else(|| {
                let names = available_names(&available);
                syn::Error::new(
                    name.span(),
                    format!("unknown interface `{value}`; available interfaces: {names}"),
                )
            })
        })
        .collect()
}

struct Generator<'a> {
    resolve: &'a Resolve,
    errors: std::collections::HashSet<wit_parser::TypeId>,
    selected: std::collections::HashMap<InterfaceId, std::collections::HashSet<wit_parser::TypeId>>,
    with: std::collections::HashMap<PackageId, Path>,
}

impl Generator<'_> {
    fn interface(&self, name: &str, id: InterfaceId) -> syn::Result<TokenStream> {
        let module = rust_ident(&name.to_snake_case())?;
        let package_id = self.resolve.interfaces[id]
            .package
            .ok_or_else(|| syn::Error::new(Span::call_site(), "interface has no package"))?;
        let interface = self.resolve.packages[package_id].name.interface_id(name);
        let types = self.resolve.interfaces[id]
            .types
            .iter()
            .filter(|(_, type_id)| self.selected[&id].contains(*type_id))
            .filter(|(_, type_id)| {
                !matches!(
                    self.resolve.types[**type_id].kind,
                    wit_parser::TypeDefKind::Resource
                )
            })
            .map(|(export, type_id)| {
                let definition = &self.resolve.types[*type_id];
                if definition.owner == wit_parser::TypeOwner::Interface(id) {
                    self.type_definition(export, *type_id)
                } else {
                    let name = rust_ident(&export.to_upper_camel_case())?;
                    let target = self.named_type(*type_id)?;
                    if definition.name.as_deref() == Some(export) {
                        Ok(quote!(pub use #target;))
                    } else {
                        Ok(quote!(pub use #target as #name;))
                    }
                }
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let calls = self.resolve.interfaces[id]
            .functions
            .values()
            .map(|function| self.typed_call(name, function))
            .collect::<syn::Result<Vec<_>>>()?;
        let host = self.host_trait(name, self.resolve.interfaces[id].functions.values())?;
        let handle = self.handle(name, self.resolve.interfaces[id].functions.values())?;
        let provider = Self::provider(name, self.resolve.interfaces[id].functions.values())?;
        Ok(quote! {
            #[doc = concat!("Bindings for the `", #interface, "` interface.")]
            pub mod #module {
                /// The fully qualified WIT interface name.
                pub const INTERFACE: &str = #interface;
                #(#types)*
                #host
                #(#calls)*
                #handle
                #provider
            }
        })
    }

    fn type_definition(&self, name: &str, id: wit_parser::TypeId) -> syn::Result<TokenStream> {
        let ident = rust_ident(&name.to_upper_camel_case())?;
        let (definition, nominal) = match &self.resolve.types[id].kind {
            wit_parser::TypeDefKind::Type(wit_parser::Type::Id(target))
                if self.resolve.types[*target].owner != self.resolve.types[id].owner =>
            {
                let target_path = self.named_type(*target)?;
                let target_name = self.resolve.types[*target]
                    .name
                    .as_deref()
                    .ok_or_else(|| Self::unsupported(name, "anonymous imported type"))?;
                let target_ident = rust_ident(&target_name.to_upper_camel_case())?;
                let tokens = if ident == target_ident {
                    quote!(pub use #target_path;)
                } else {
                    quote!(pub use #target_path as #ident;)
                };
                (tokens, false)
            }
            wit_parser::TypeDefKind::Record(record) => (self.record(name, record)?, true),
            wit_parser::TypeDefKind::Enum(enum_) => (Self::enum_(name, enum_)?, true),
            wit_parser::TypeDefKind::Flags(flags) => (Self::flags(name, flags)?, true),
            wit_parser::TypeDefKind::Variant(variant) => (self.variant(name, variant)?, true),
            kind => {
                let ty = self.type_kind(kind, name)?;
                (
                    quote! {
                        #[doc = concat!("The WIT `", #name, "` type.")]
                        pub type #ident = #ty;
                    },
                    false,
                )
            }
        };
        let error = nominal
            .then(|| self.error_display(name, id, &ident))
            .transpose()?;
        Ok(quote!(#definition #error))
    }
}

pub(crate) fn rust_ident(name: &str) -> syn::Result<Ident> {
    if matches!(name, "self" | "Self" | "super" | "crate") {
        return syn::parse_str(&format!("{name}_"));
    }
    syn::parse_str(name)
        .or_else(|_| syn::parse_str(&format!("r#{name}")))
        .map_err(|_| syn::Error::new(Span::call_site(), format!("invalid Rust name `{name}`")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use proc_macro2::Span;
    use wit_parser::Resolve;

    #[test]
    fn call_and_parameter_names_reserve_generated_bindings() {
        assert_eq!(
            [
                super::collisions::call_ident("notes", "host").unwrap(),
                super::collisions::call_ident("notes", "notes").unwrap(),
                super::collisions::call_ident("notes", "provider").unwrap(),
            ]
            .map(|name| name.to_string()),
            ["Host_", "Notes_", "Provider"]
        );
        assert_eq!(super::collisions::parameter_ident("cx").unwrap(), "cx_");
        assert_eq!(super::collisions::parameter_ident("call").unwrap(), "call");
        assert_eq!(super::collisions::method_ident("with").unwrap(), "with_");
        assert_eq!(
            super::collisions::method_ident("within").unwrap(),
            "within_"
        );
        assert_eq!(super::collisions::method_ident("clone").unwrap(), "clone_");
        assert_eq!(
            super::collisions::method_ident("from-app").unwrap(),
            "from_app_"
        );
    }

    #[test]
    fn unchanged_use_omits_redundant_alias() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../wasm-junction/tests/fixtures/dependencies/wit");
        let mut resolve = Resolve::default();
        let (package, _) = resolve.push_path(path).unwrap();
        let tokens = super::generate(&resolve, package, None, &[], Span::call_site())
            .unwrap()
            .to_string();
        assert!(
            tokens.contains("pub use super :: types :: Author ;"),
            "{tokens}"
        );
        assert!(!tokens.contains("Author as Author"));
    }
}
