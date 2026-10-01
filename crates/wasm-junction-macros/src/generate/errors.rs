use std::collections::HashSet;

use wit_parser::{InterfaceId, Resolve, Type, TypeDefKind, TypeId};

use super::walk::{self, Position};

pub(super) fn find(resolve: &Resolve, roots: &[InterfaceId]) -> syn::Result<HashSet<TypeId>> {
    let mut errors = HashSet::new();
    walk::interfaces(resolve, roots.iter().copied(), |type_use| {
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
