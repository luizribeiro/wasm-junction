use std::collections::{HashMap, HashSet};

use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::Span;
use wit_parser::{InterfaceId, PackageId, Resolve, Type, TypeDefKind, TypeId, TypeOwner};

use super::{rust_ident, walk};

pub(super) fn call_ident(interface: &str, name: &str) -> syn::Result<proc_macro2::Ident> {
    let fixed = fixed_names(interface);
    generated_ident(&name.to_upper_camel_case(), &fixed)
}

pub(super) fn method_ident(name: &str) -> syn::Result<proc_macro2::Ident> {
    generated_ident(
        &name.to_snake_case(),
        &[
            "clone".to_owned(),
            "from_app".to_owned(),
            "with".to_owned(),
            "within".to_owned(),
        ],
    )
}

pub(super) fn parameter_ident(name: &str) -> syn::Result<proc_macro2::Ident> {
    generated_ident(&name.to_snake_case(), &["cx".to_owned()])
}

fn generated_ident(name: &str, reserved: &[String]) -> syn::Result<proc_macro2::Ident> {
    if reserved.iter().any(|reserved| reserved == name) {
        rust_ident(&format!("{name}_"))
    } else {
        rust_ident(name)
    }
}

fn fixed_names(interface: &str) -> [String; 4] {
    [
        "Host".to_owned(),
        "HostProvider".to_owned(),
        "INTERFACE".to_owned(),
        interface.to_upper_camel_case(),
    ]
}

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
        let interface_name = interface
            .name
            .as_deref()
            .ok_or_else(|| syn::Error::new(span, "reachable interface has no WIT name"))?;
        for fixed in fixed_names(interface_name) {
            names.insert(fixed.clone(), format!("generated `{fixed}`"));
        }
        for (name, id) in &interface.types {
            if !types.contains(id) {
                continue;
            }
            let rust = rust_ident(&name.to_upper_camel_case())?.to_string();
            unique(&mut names, &rust, name, "type", span)?;
        }
        for function in interface.functions.values() {
            let rust = call_ident(interface_name, &function.name)?.to_string();
            unique(&mut names, &rust, &function.name, "function", span)?;
        }
    }
    for id in types {
        let kind = &resolve.types[id].kind;
        match kind {
            TypeDefKind::Record(record) => check_members(
                record.fields.iter().map(|field| field.name.as_str()),
                false,
                "field",
                span,
            )?,
            TypeDefKind::Variant(variant) => check_members(
                variant.cases.iter().map(|case| case.name.as_str()),
                true,
                "case",
                span,
            )?,
            TypeDefKind::Enum(enum_) => check_members(
                enum_.cases.iter().map(|case| case.name.as_str()),
                true,
                "case",
                span,
            )?,
            TypeDefKind::Flags(flags) => check_members(
                flags.flags.iter().map(|flag| flag.name.as_str()),
                false,
                "flag",
                span,
            )?,
            _ => {}
        }
    }
    Ok(())
}

fn check_members<'a>(
    items: impl Iterator<Item = &'a str>,
    upper_camel: bool,
    kind: &str,
    span: Span,
) -> syn::Result<()> {
    let mut names = HashMap::new();
    for wit in items {
        let converted = if upper_camel {
            wit.to_upper_camel_case()
        } else {
            wit.to_snake_case()
        };
        let rust = rust_ident(&converted)?.to_string();
        unique(&mut names, &rust, wit, kind, span)?;
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

    #[test]
    fn member_collisions_name_both_members() {
        let error = collision("tests/fixtures/collisions/cases/wit");
        assert!(error.contains("WIT cases `http2` and `http-2`"));
        for kind in ["field", "flag"] {
            let error = super::check_members(
                ["hello__world", "hello-world"].into_iter(),
                false,
                kind,
                Span::call_site(),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(&format!("WIT {kind}s `hello__world` and `hello-world`")));
        }
    }
}
