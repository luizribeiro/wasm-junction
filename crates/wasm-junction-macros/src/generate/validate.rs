use proc_macro2::Span;
use wit_parser::{InterfaceId, Resolve, Type, TypeDefKind};

use super::walk;

pub(super) fn interfaces(resolve: &Resolve, roots: &[InterfaceId], span: Span) -> syn::Result<()> {
    walk::interfaces(resolve, roots.iter().copied(), |type_use| {
        let shape = match type_use.ty {
            Type::ErrorContext => Some("error-context"),
            Type::Id(id) => match resolve.types[id].kind {
                TypeDefKind::Resource | TypeDefKind::Handle(_) => Some("resource"),
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
    })
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
    fn unsupported_errors_name_resources_futures_and_streams() {
        for (path, expected) in [
            (
                "../wasm-junction/tests/fixtures/unsupported/wit",
                "WIT resource in `session`",
            ),
            ("tests/fixtures/future", "WIT future in `delayed`"),
            ("tests/fixtures/stream", "WIT stream in `chunks`"),
        ] {
            assert!(validation_error(path).contains(expected));
        }
    }
}
