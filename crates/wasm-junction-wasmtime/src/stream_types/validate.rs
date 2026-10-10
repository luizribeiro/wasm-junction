use wasmtime::Engine;
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Type};

use super::{StreamTypeVisitor, visit_stream_type};
use crate::stream_values::StreamValue;

pub(super) fn validate_stream_type(ty: &Type) -> Result<(), wasmtime::Error> {
    visit_stream_type(ty, Validate)
}

struct Validate;

impl StreamTypeVisitor for Validate {
    type Output = ();

    fn visit<T: StreamValue>(self) -> Result<Self::Output, wasmtime::Error> {
        Ok(())
    }
}

pub(crate) fn validate_component_streams(
    component: &Component,
    engine: &Engine,
    static_stream_interfaces: &[&str],
) -> Result<(), wasmtime::Error> {
    let component = component.component_type();
    component
        .imports(engine)
        .chain(component.exports(engine))
        .try_for_each(|(name, item)| {
            validate_item(name, item.ty, engine, static_stream_interfaces, false)
        })
}

fn validate_item(
    name: &str,
    item: ComponentItem,
    engine: &Engine,
    static_stream_interfaces: &[&str],
    static_streams: bool,
) -> Result<(), wasmtime::Error> {
    match item {
        ComponentItem::ComponentFunc(function) => function
            .params()
            .map(|(_, ty)| ty)
            .chain(function.results())
            .try_for_each(|ty| validate_value(&ty, static_streams)),
        ComponentItem::ComponentInstance(instance) => {
            let static_streams = static_streams || static_stream_interfaces.contains(&name);
            instance.exports(engine).try_for_each(|(export, item)| {
                validate_item(
                    export,
                    item.ty,
                    engine,
                    static_stream_interfaces,
                    static_streams,
                )
            })
        }
        _ => Ok(()),
    }
}

fn validate_value(ty: &Type, static_streams: bool) -> Result<(), wasmtime::Error> {
    match ty {
        Type::Stream(stream) => {
            let item = stream
                .ty()
                .ok_or_else(|| wasmtime::Error::msg("stream has no item type"))?;
            if !static_streams {
                validate_stream_type(&item)?;
            }
        }
        Type::List(ty) => validate_value(&ty.ty(), static_streams)?,
        Type::Map(ty) => {
            validate_value(&ty.key(), static_streams)?;
            validate_value(&ty.value(), static_streams)?;
        }
        Type::Record(ty) => ty
            .fields()
            .try_for_each(|field| validate_value(&field.ty, static_streams))?,
        Type::Tuple(ty) => ty
            .types()
            .try_for_each(|ty| validate_value(&ty, static_streams))?,
        Type::Variant(ty) => ty
            .cases()
            .filter_map(|case| case.ty)
            .try_for_each(|ty| validate_value(&ty, static_streams))?,
        Type::Option(ty) => validate_value(&ty.ty(), static_streams)?,
        Type::Result(ty) => ty
            .ok()
            .into_iter()
            .chain(ty.err())
            .try_for_each(|ty| validate_value(&ty, static_streams))?,
        Type::Future(ty) => ty
            .ty()
            .into_iter()
            .try_for_each(|ty| validate_value(&ty, static_streams))?,
        Type::FixedLengthList(ty) => validate_value(&ty.ty(), static_streams)?,
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_types::test_support::{component_with_record_stream_import, item_type};

    #[test]
    fn validation_uses_the_static_type_resolver() {
        validate_stream_type(&item_type("", "list<option<string>>")).unwrap();

        for (item, shape) in [
            ("tuple<u32>", "tuple"),
            ("result<string, string>", "result"),
            ("option<list<option<u32>>>", "option"),
        ] {
            let error = validate_stream_type(&item_type("", item)).unwrap_err();
            assert!(error.to_string().contains(&format!("item type `{shape}`")));
        }
    }

    #[test]
    fn declared_static_record_stream_is_accepted() {
        let (engine, component) =
            component_with_record_stream_import("wasi:filesystem@0.3.0", "types");

        validate_component_streams(&component, &engine, &["wasi:filesystem/types@0.3.0"]).unwrap();
    }

    #[test]
    fn record_stream_in_another_interface_is_refused() {
        let (engine, component) =
            component_with_record_stream_import("example:other@1.0.0", "types");

        let error =
            validate_component_streams(&component, &engine, &["wasi:filesystem/types@0.3.0"])
                .unwrap_err();

        assert!(error.to_string().contains("item type `record`"));
    }
}
