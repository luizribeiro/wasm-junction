use proc_macro2::Span;
use wit_parser::{InterfaceId, Resolve, Type, TypeDefKind};

use super::walk;

pub(super) fn interfaces(resolve: &Resolve, roots: &[InterfaceId], span: Span) -> syn::Result<()> {
    walk::interfaces(resolve, roots.iter().copied(), |type_use| {
        let shape = match type_use.ty {
            Type::ErrorContext => Some("error-context"),
            Type::Id(id) => match resolve.types[id].kind {
                TypeDefKind::Future(_) => Some("future"),
                TypeDefKind::Stream(_) => Some("stream"),
                _ => None,
            },
            _ => None,
        };
        if let Some(shape) = shape {
            return Err(syn::Error::new(
                span,
                format!("WIT {shape} in `{}` is not supported yet", type_use.item),
            ));
        }
        Ok(())
    })?;
    for interface in roots {
        for function in resolve.interfaces[*interface].functions.values() {
            for ty in function
                .params
                .iter()
                .map(|param| param.ty)
                .chain(function.result)
            {
                validate_resource_shape(resolve, ty, &function.name, true, span)?;
            }
        }
    }
    Ok(())
}

fn validate_resource_shape(
    resolve: &Resolve,
    ty: Type,
    function: &str,
    allow_wrapper: bool,
    span: Span,
) -> syn::Result<()> {
    let Type::Id(id) = ty else { return Ok(()) };
    match &resolve.types[id].kind {
        TypeDefKind::Resource | TypeDefKind::Handle(_) => Ok(()),
        TypeDefKind::Type(ty) => {
            validate_resource_shape(resolve, *ty, function, allow_wrapper, span)
        }
        TypeDefKind::Option(ty) if allow_wrapper => {
            validate_resource_shape(resolve, *ty, function, false, span)
        }
        TypeDefKind::Result(result) if allow_wrapper => {
            for ty in result.ok.into_iter().chain(result.err) {
                validate_resource_shape(resolve, ty, function, false, span)?;
            }
            Ok(())
        }
        _ => {
            let mut contains_resource = false;
            walk::type_(resolve, ty, function, |type_use| {
                let Type::Id(id) = type_use.ty else {
                    return Ok(());
                };
                contains_resource |= matches!(
                    resolve.types[id].kind,
                    TypeDefKind::Resource | TypeDefKind::Handle(_)
                );
                Ok(())
            })?;
            if contains_resource {
                Err(syn::Error::new(
                    span,
                    format!("WIT resource nested in `{function}` is not supported yet"),
                ))
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use proc_macro2::Span;
    use wit_parser::Resolve;

    fn validation_error(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        let mut resolve = Resolve::default();
        let (package, _) = resolve.push_path(path).unwrap();
        let roots = resolve.packages[package]
            .interfaces
            .values()
            .copied()
            .collect::<Vec<_>>();
        super::interfaces(&resolve, &roots, Span::call_site())
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn unsupported_errors_name_futures_and_streams() {
        for (path, expected) in [
            ("tests/fixtures/future", "WIT future in `delayed`"),
            ("tests/fixtures/stream", "WIT stream in `chunks`"),
        ] {
            assert!(validation_error(path).contains(expected));
        }
    }

    #[test]
    fn nested_resources_name_the_function() {
        let error = validation_error("tests/fixtures/resource-nesting");
        assert!(error.contains("resource nested in `bad`"), "{error}");
    }
}
