use std::collections::HashSet;

use wit_parser::{PackageId, Resolve, Type, TypeDefKind, TypeId};

pub(super) fn find(resolve: &Resolve, package_id: PackageId) -> HashSet<TypeId> {
    let mut errors = HashSet::new();
    let mut seen = HashSet::new();
    for interface_id in resolve.packages[package_id].interfaces.values() {
        let interface = &resolve.interfaces[*interface_id];
        for id in interface.types.values() {
            visit(resolve, Type::Id(*id), &mut errors, &mut seen);
        }
        for function in interface.functions.values() {
            for param in &function.params {
                visit(resolve, param.ty, &mut errors, &mut seen);
            }
            if let Some(result) = function.result {
                visit(resolve, result, &mut errors, &mut seen);
            }
        }
    }
    errors
}

fn visit(resolve: &Resolve, ty: Type, errors: &mut HashSet<TypeId>, seen: &mut HashSet<TypeId>) {
    let Type::Id(id) = ty else { return };
    if !seen.insert(id) {
        return;
    }
    match &resolve.types[id].kind {
        TypeDefKind::Record(record) => {
            for field in &record.fields {
                visit(resolve, field.ty, errors, seen);
            }
        }
        TypeDefKind::Variant(variant) => {
            for case in &variant.cases {
                if let Some(ty) = case.ty {
                    visit(resolve, ty, errors, seen);
                }
            }
        }
        TypeDefKind::Tuple(tuple) => {
            for ty in &tuple.types {
                visit(resolve, *ty, errors, seen);
            }
        }
        TypeDefKind::Option(ty) | TypeDefKind::List(ty) | TypeDefKind::Type(ty) => {
            visit(resolve, *ty, errors, seen);
        }
        TypeDefKind::Result(result) => {
            if let Some(ok) = result.ok {
                visit(resolve, ok, errors, seen);
            }
            if let Some(err) = result.err {
                mark_error(resolve, err, errors);
                visit(resolve, err, errors, seen);
            }
        }
        _ => {}
    }
}

fn mark_error(resolve: &Resolve, ty: Type, errors: &mut HashSet<TypeId>) {
    let Type::Id(id) = ty else { return };
    if let TypeDefKind::Type(inner) = resolve.types[id].kind {
        mark_error(resolve, inner, errors);
    } else if resolve.types[id].name.is_some() {
        errors.insert(id);
    }
}
