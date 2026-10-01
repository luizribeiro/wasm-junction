use std::collections::{HashMap, HashSet};

use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::Span;
use wit_parser::{InterfaceId, PackageId, Resolve, Type, TypeId, TypeOwner};

use super::{rust_ident, walk};

pub(super) fn check(resolve: &Resolve, package: PackageId, span: Span) -> syn::Result<()> {
    let (interfaces, types) = reachable(resolve, package)?;
    let mut names = HashMap::new();
    for id in &interfaces {
        let interface = &resolve.interfaces[*id];
        let name = interface
            .name
            .as_deref()
            .ok_or_else(|| syn::Error::new(span, "reachable interface has no WIT name"))?;
        let rust = rust_ident(&name.to_snake_case())?.to_string();
        let package = interface
            .package
            .ok_or_else(|| syn::Error::new(span, "reachable interface has no WIT package"))?;
        let wit = resolve.packages[package].name.interface_id(name);
        unique(&mut names, &rust, &wit, "interface", span)?;
    }

    for interface_id in interfaces {
        names.clear();
        let interface = &resolve.interfaces[interface_id];
        for (name, id) in &interface.types {
            if !types.contains(id) {
                continue;
            }
            let rust = rust_ident(&name.to_upper_camel_case())?.to_string();
            unique(&mut names, &rust, name, "type", span)?;
        }
    }
    Ok(())
}

fn reachable(
    resolve: &Resolve,
    package: PackageId,
) -> syn::Result<(HashSet<InterfaceId>, HashSet<TypeId>)> {
    let mut interfaces = resolve.packages[package]
        .interfaces
        .values()
        .copied()
        .collect::<HashSet<_>>();
    let mut types = HashSet::new();
    walk::package(resolve, package, |type_use| {
        let Type::Id(id) = type_use.ty else {
            return Ok(());
        };
        types.insert(id);
        if let TypeOwner::Interface(owner) = resolve.types[id].owner {
            interfaces.insert(owner);
        }
        Ok(())
    })?;
    Ok((interfaces, types))
}

fn unique(
    names: &mut HashMap<String, String>,
    rust: &str,
    wit: &str,
    kind: &str,
    span: Span,
) -> syn::Result<()> {
    if let Some(previous) = names.insert(rust.to_owned(), wit.to_owned()) {
        return Err(syn::Error::new(
            span,
            format!("WIT {kind}s `{previous}` and `{wit}` both generate Rust identifier `{rust}`"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use proc_macro2::Span;
    use wit_parser::Resolve;

    fn collision(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        let mut resolve = Resolve::default();
        let (package, _) = resolve.push_path(path).unwrap();
        super::check(&resolve, package, Span::call_site())
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn interface_collision_names_both_interfaces() {
        let error = collision("tests/fixtures/collisions/interfaces/wit");
        assert!(error.contains("test:local/shared-types"));
        assert!(error.contains("test:dependency/shared-types"));
    }

    #[test]
    fn type_collision_names_both_types() {
        let error = collision("tests/fixtures/collisions/types/wit");
        assert!(error.contains("`http2` and `http-2`"));
        assert!(error.contains("`Http2`"));
    }
}
