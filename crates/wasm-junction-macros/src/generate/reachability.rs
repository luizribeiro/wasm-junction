use std::collections::{HashMap, HashSet};

use wit_parser::{InterfaceId, PackageId, Resolve, Type, TypeDefKind, TypeId, TypeOwner};

pub(super) fn find(resolve: &Resolve, package: PackageId) -> HashMap<InterfaceId, HashSet<TypeId>> {
    let mut selected = HashMap::new();
    let mut seen = HashSet::new();
    for interface_id in resolve.packages[package].interfaces.values() {
        let interface = &resolve.interfaces[*interface_id];
        selected.entry(*interface_id).or_default();
        for id in interface.types.values() {
            visit(resolve, Type::Id(*id), &mut selected, &mut seen);
        }
        for function in interface.functions.values() {
            for param in &function.params {
                visit(resolve, param.ty, &mut selected, &mut seen);
            }
            if let Some(result) = function.result {
                visit(resolve, result, &mut selected, &mut seen);
            }
        }
    }
    selected
}

fn visit(
    resolve: &Resolve,
    ty: Type,
    selected: &mut HashMap<InterfaceId, HashSet<TypeId>>,
    seen: &mut HashSet<TypeId>,
) {
    let Type::Id(id) = ty else { return };
    if !seen.insert(id) {
        return;
    }
    if let TypeOwner::Interface(owner) = resolve.types[id].owner {
        selected.entry(owner).or_default().insert(id);
    }
    match &resolve.types[id].kind {
        TypeDefKind::Record(record) => {
            for field in &record.fields {
                visit(resolve, field.ty, selected, seen);
            }
        }
        TypeDefKind::Variant(variant) => {
            for case in &variant.cases {
                if let Some(ty) = case.ty {
                    visit(resolve, ty, selected, seen);
                }
            }
        }
        TypeDefKind::Tuple(tuple) => {
            for ty in &tuple.types {
                visit(resolve, *ty, selected, seen);
            }
        }
        TypeDefKind::Option(ty) | TypeDefKind::List(ty) | TypeDefKind::Type(ty) => {
            visit(resolve, *ty, selected, seen);
        }
        TypeDefKind::Result(result) => {
            if let Some(ty) = result.ok {
                visit(resolve, ty, selected, seen);
            }
            if let Some(ty) = result.err {
                visit(resolve, ty, selected, seen);
            }
        }
        _ => {}
    }
}
