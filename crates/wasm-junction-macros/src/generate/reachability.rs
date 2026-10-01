use std::collections::{HashMap, HashSet};

use wit_parser::{InterfaceId, PackageId, Resolve, Type, TypeId, TypeOwner};

use super::walk;

pub(super) fn find(
    resolve: &Resolve,
    package: PackageId,
) -> syn::Result<HashMap<InterfaceId, HashSet<TypeId>>> {
    let mut selected: HashMap<InterfaceId, HashSet<TypeId>> = HashMap::new();
    for interface in resolve.packages[package].interfaces.values() {
        selected.entry(*interface).or_default();
    }
    walk::package(resolve, package, |type_use| {
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
