use std::any::Any;

use wasm_junction_core::Val;
use wasmtime::component::Type;

use super::{StreamValue, required, shape};
use crate::engine::StoreData;

impl<T: StreamValue> StreamValue for Vec<T> {
    fn into_val(
        self,
        ty: Option<&Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Option<Val>, wasmtime::Error> {
        let item = list_item(ty)?;
        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<u8>() {
            return (Box::new(self) as Box<dyn Any>)
                .downcast::<Vec<u8>>()
                .map(|values| Some(Val::Bytes(*values)))
                .map_err(|_| wasmtime::Error::msg("invalid list<u8> stream item"));
        }
        let values = self
            .into_iter()
            .map(|value| required(value.into_val(Some(&item), store)?, "list item"))
            .collect::<Result<_, _>>()?;
        Ok(Some(Val::List(values)))
    }

    fn from_val(
        value: Option<Val>,
        ty: Option<&Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Self, wasmtime::Error> {
        let item = list_item(ty)?;
        let values = match value {
            Some(Val::List(values)) => values,
            Some(Val::Bytes(values))
                if std::any::TypeId::of::<T>() == std::any::TypeId::of::<u8>() =>
            {
                return (Box::new(values) as Box<dyn Any>)
                    .downcast::<Self>()
                    .map(|values| *values)
                    .map_err(|_| wasmtime::Error::msg("invalid list<u8> stream item"));
            }
            value => return Err(shape("list", value.as_ref())),
        };
        values
            .into_iter()
            .map(|value| T::from_val(Some(value), Some(&item), store))
            .collect()
    }
}

impl<T: StreamValue> StreamValue for Option<T> {
    fn into_val(
        self,
        ty: Option<&Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Option<Val>, wasmtime::Error> {
        let item = option_item(ty)?;
        Ok(Some(Val::Option(
            self.map(|value| {
                value
                    .into_val(Some(&item), store)
                    .and_then(|value| required(value, "option item"))
                    .map(Box::new)
            })
            .transpose()?,
        )))
    }

    fn from_val(
        value: Option<Val>,
        ty: Option<&Type>,
        store: &mut wasmtime::StoreContextMut<'_, StoreData>,
    ) -> Result<Self, wasmtime::Error> {
        let item = option_item(ty)?;
        match value {
            Some(Val::Option(value)) => value
                .map(|value| T::from_val(Some(*value), Some(&item), store))
                .transpose(),
            value => Err(shape("option", value.as_ref())),
        }
    }
}

fn list_item(ty: Option<&Type>) -> Result<Type, wasmtime::Error> {
    match ty {
        Some(Type::List(ty)) => Ok(ty.ty()),
        _ => Err(wasmtime::Error::msg("expected list stream item")),
    }
}

fn option_item(ty: Option<&Type>) -> Result<Type, wasmtime::Error> {
    match ty {
        Some(Type::Option(ty)) => Ok(ty.ty()),
        _ => Err(wasmtime::Error::msg("expected option stream item")),
    }
}

#[cfg(test)]
mod tests {
    use wasmtime::AsContextMut;

    use super::*;
    use crate::stream_types::test_support::item_type;
    use crate::stream_values::test_store;

    #[test]
    fn list_and_option_items_convert_recursively() {
        let mut store = test_store();
        let mut context = store.as_context_mut();

        let bytes = item_type("", "list<u8>");
        assert_eq!(
            vec![1_u8, 2].into_val(Some(&bytes), &mut context).unwrap(),
            Some(Val::Bytes(vec![1, 2]))
        );
        assert_eq!(
            Vec::<u8>::from_val(Some(Val::Bytes(vec![3, 4])), Some(&bytes), &mut context).unwrap(),
            [3, 4]
        );

        let nested = item_type("", "list<option<string>>");
        let value = vec![Some("one".to_owned()), None];
        let encoded = value.clone().into_val(Some(&nested), &mut context).unwrap();
        assert_eq!(
            Vec::<Option<String>>::from_val(encoded, Some(&nested), &mut context).unwrap(),
            value
        );
    }
}
