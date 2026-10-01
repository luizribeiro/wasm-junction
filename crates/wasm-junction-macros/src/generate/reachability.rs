use std::collections::{HashMap, HashSet};

use wit_parser::{InterfaceId, Resolve, Type, TypeId, TypeOwner};

use super::walk;

pub(super) fn find(
    resolve: &Resolve,
    roots: &[InterfaceId],
) -> syn::Result<HashMap<InterfaceId, HashSet<TypeId>>> {
    let mut selected: HashMap<InterfaceId, HashSet<TypeId>> = HashMap::new();
    for interface in roots {
        selected.entry(*interface).or_default();
    }
    walk::interfaces(resolve, roots.iter().copied(), |type_use| {
        let Type::Id(id) = type_use.ty else {
            return Ok(());
        };
        if let TypeOwner::Interface(owner) = resolve.types[id].owner {
            selected.entry(owner).or_default().insert(id);
        }
        Ok(())
    })?;
    Ok(selected)
}
