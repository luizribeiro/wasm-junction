use std::collections::HashSet;

use proc_macro2::Span;
use wit_parser::{PackageId, Resolve, Type, TypeDefKind, TypeId};

pub(super) fn package(resolve: &Resolve, package: PackageId) -> syn::Result<()> {
    for interface_id in resolve.packages[package].interfaces.values() {
        let interface = &resolve.interfaces[*interface_id];
        for (name, id) in &interface.types {
            visit(resolve, Type::Id(*id), name, &mut HashSet::new())?;
        }
        for function in interface.functions.values() {
            for param in &function.params {
                visit(resolve, param.ty, &function.name, &mut HashSet::new())?;
            }
            if let Some(result) = function.result {
                visit(resolve, result, &function.name, &mut HashSet::new())?;
            }
        }
    }
    Ok(())
}

fn visit(resolve: &Resolve, ty: Type, item: &str, seen: &mut HashSet<TypeId>) -> syn::Result<()> {
    let Type::Id(id) = ty else {
        return if ty == Type::ErrorContext {
            Err(unsupported(item, "error-context"))
        } else {
            Ok(())
        };
    };
    if !seen.insert(id) {
        return Ok(());
    }
    match &resolve.types[id].kind {
        TypeDefKind::Resource | TypeDefKind::Handle(_) => Err(unsupported(item, "resource")),
        TypeDefKind::Future(_) => Err(unsupported(item, "future")),
        TypeDefKind::Stream(_) => Err(unsupported(item, "stream")),
        TypeDefKind::Record(record) => {
            for field in &record.fields {
                visit(resolve, field.ty, item, seen)?;
            }
            Ok(())
        }
        TypeDefKind::Variant(variant) => {
            for case in &variant.cases {
                if let Some(ty) = case.ty {
                    visit(resolve, ty, item, seen)?;
                }
            }
            Ok(())
        }
        TypeDefKind::Tuple(tuple) => {
            for ty in &tuple.types {
                visit(resolve, *ty, item, seen)?;
            }
            Ok(())
        }
        TypeDefKind::Option(ty) | TypeDefKind::List(ty) | TypeDefKind::Type(ty) => {
            visit(resolve, *ty, item, seen)
        }
        TypeDefKind::Result(result) => {
            if let Some(ty) = result.ok {
                visit(resolve, ty, item, seen)?;
            }
            if let Some(ty) = result.err {
                visit(resolve, ty, item, seen)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn unsupported(item: &str, shape: &str) -> syn::Error {
    syn::Error::new(
        Span::call_site(),
        format!("WIT {shape} in `{item}` is not supported yet"),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use wit_parser::Resolve;

    #[test]
    fn unsupported_error_names_the_item() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../wasm-junction/tests/fixtures/unsupported/wit");
        let mut resolve = Resolve::default();
        let (package, _) = resolve.push_path(path).unwrap();
        let error = super::package(&resolve, package).unwrap_err();
        assert_eq!(
            error.to_string(),
            "WIT resource in `session` is not supported yet"
        );
    }
}
