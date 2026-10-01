use std::collections::HashSet;

use wit_parser::{PackageId, Resolve, Type, TypeDefKind, TypeId};

use super::walk::{self, Position};

pub(super) fn find(resolve: &Resolve, package: PackageId) -> syn::Result<HashSet<TypeId>> {
    let mut errors = HashSet::new();
    walk::package(resolve, package, |type_use| {
        if type_use.position != Position::Error {
            return Ok(());
        }
        let Type::Id(id) = type_use.ty else {
            return Ok(());
        };
        if matches!(
            resolve.types[id].kind,
            TypeDefKind::Record(_)
                | TypeDefKind::Variant(_)
                | TypeDefKind::Enum(_)
                | TypeDefKind::Flags(_)
        ) && resolve.types[id].name.is_some()
        {
            errors.insert(id);
        }
        Ok(())
    })?;
    Ok(errors)
}
