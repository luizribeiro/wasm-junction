use std::collections::HashSet;

use wit_parser::{PackageId, Resolve, Type, TypeDefKind, TypeId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Position {
    Other,
    Error,
}

#[derive(Clone, Copy)]
pub(super) struct TypeUse<'a> {
    pub(super) ty: Type,
    pub(super) item: &'a str,
    pub(super) position: Position,
}

pub(super) fn package(
    resolve: &Resolve,
    package: PackageId,
    mut callback: impl FnMut(TypeUse<'_>) -> syn::Result<()>,
) -> syn::Result<()> {
    let mut seen = HashSet::new();
    for interface_id in resolve.packages[package].interfaces.values() {
        let interface = &resolve.interfaces[*interface_id];
        for (name, id) in &interface.types {
            visit(
                resolve,
                Type::Id(*id),
                name,
                Position::Other,
                &mut seen,
                &mut callback,
            )?;
        }
        for function in interface.functions.values() {
            for param in &function.params {
                visit(
                    resolve,
                    param.ty,
                    &function.name,
                    Position::Other,
                    &mut seen,
                    &mut callback,
                )?;
            }
            if let Some(result) = function.result {
                visit(
                    resolve,
                    result,
                    &function.name,
                    Position::Other,
                    &mut seen,
                    &mut callback,
                )?;
            }
        }
    }
    Ok(())
}

fn visit(
    resolve: &Resolve,
    ty: Type,
    item: &str,
    position: Position,
    seen: &mut HashSet<TypeId>,
    callback: &mut impl FnMut(TypeUse<'_>) -> syn::Result<()>,
) -> syn::Result<()> {
    callback(TypeUse { ty, item, position })?;
    let Type::Id(id) = ty else { return Ok(()) };
    if !seen.insert(id) {
        return Ok(());
    }
    let mut nested = |ty| visit(resolve, ty, item, Position::Other, seen, callback);
    match &resolve.types[id].kind {
        TypeDefKind::Record(record) => {
            for field in &record.fields {
                nested(field.ty)?;
            }
        }
        TypeDefKind::Variant(variant) => {
            for case in &variant.cases {
                if let Some(ty) = case.ty {
                    nested(ty)?;
                }
            }
        }
        TypeDefKind::Tuple(tuple) => {
            for ty in &tuple.types {
                nested(*ty)?;
            }
        }
        TypeDefKind::Option(ty)
        | TypeDefKind::List(ty)
        | TypeDefKind::Future(Some(ty))
        | TypeDefKind::Stream(Some(ty))
        | TypeDefKind::FixedLengthList(ty, _) => nested(*ty)?,
        TypeDefKind::Map(key, value) => {
            nested(*key)?;
            nested(*value)?;
        }
        TypeDefKind::Type(ty) => {
            visit(resolve, *ty, item, position, seen, callback)?;
        }
        TypeDefKind::Result(result) => {
            if let Some(ty) = result.ok {
                nested(ty)?;
            }
            if let Some(ty) = result.err {
                visit(resolve, ty, item, Position::Error, seen, callback)?;
            }
        }
        _ => {}
    }
    Ok(())
}
