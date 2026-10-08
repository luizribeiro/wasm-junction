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
) -> Result<(), wasmtime::Error> {
    let component = component.component_type();
    component
        .imports(engine)
        .chain(component.exports(engine))
        .try_for_each(|(_, item)| validate_item(item.ty, engine))
}

fn validate_item(item: ComponentItem, engine: &Engine) -> Result<(), wasmtime::Error> {
    match item {
        ComponentItem::ComponentFunc(function) => function
            .params()
            .map(|(_, ty)| ty)
            .chain(function.results())
            .try_for_each(|ty| validate_value(&ty)),
        ComponentItem::ComponentInstance(instance) => instance
            .exports(engine)
            .try_for_each(|(_, item)| validate_item(item.ty, engine)),
        _ => Ok(()),
    }
}

fn validate_value(ty: &Type) -> Result<(), wasmtime::Error> {
    match ty {
        Type::Stream(stream) => validate_stream_type(
            &stream
                .ty()
                .ok_or_else(|| wasmtime::Error::msg("stream has no item type"))?,
        )?,
        Type::List(ty) => validate_value(&ty.ty())?,
        Type::Map(ty) => {
            validate_value(&ty.key())?;
            validate_value(&ty.value())?;
        }
        Type::Record(ty) => ty
            .fields()
            .try_for_each(|field| validate_value(&field.ty))?,
        Type::Tuple(ty) => ty.types().try_for_each(|ty| validate_value(&ty))?,
        Type::Variant(ty) => ty
            .cases()
            .filter_map(|case| case.ty)
            .try_for_each(|ty| validate_value(&ty))?,
        Type::Option(ty) => validate_value(&ty.ty())?,
        Type::Result(ty) => ty
            .ok()
            .into_iter()
            .chain(ty.err())
            .try_for_each(|ty| validate_value(&ty))?,
        Type::Future(ty) => ty.ty().into_iter().try_for_each(|ty| validate_value(&ty))?,
        Type::FixedLengthList(ty) => validate_value(&ty.ty())?,
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_types::test_support::item_type;

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
}
