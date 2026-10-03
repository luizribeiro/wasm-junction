use std::sync::Arc;

use wasm_junction_core::{ImportDispatcher, InvocationContext, Resource, Val, Vals};
use wasmtime::AsContextMut;
use wasmtime::bail;
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{
    Component, Linker, LinkerInstance, ResourceType, Type, Val as WasmtimeVal,
};

use crate::engine::{StoreData, lift_future, lift_resource, lower_future, lower_resource};
use crate::streams::{lift_stream, lower_stream};
use crate::values::{LiftValue, LowerValue, from_wasmtime, to_wasmtime};

pub(crate) fn define_imports(
    linker: &mut Linker<StoreData>,
    component: &Component,
) -> Result<Vec<ResourceDefinition>, wasmtime::Error> {
    let engine = linker.engine().clone();
    let mut definitions = Vec::new();
    let mut resource_types = Vec::new();
    let mut next_runtime_type = 0_u32;
    for (interface, item) in component.component_type().imports(&engine) {
        if interface.starts_with("wasi:") {
            continue;
        }
        let ComponentItem::ComponentInstance(instance) = item.ty else {
            bail!("unsupported component import `{interface}`");
        };
        let resources = instance
            .exports(&engine)
            .filter_map(|(name, item)| match item.ty {
                ComponentItem::Resource(resource) => Some((name.to_owned(), resource)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let functions = instance
            .exports(&engine)
            .filter_map(|(name, item)| match item.ty {
                ComponentItem::ComponentFunc(function) => {
                    Some((name.to_owned(), function.async_()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut instance_linker = linker.instance(interface)?;
        for (name, resource) in resources {
            let runtime_type = if let Some((_, runtime_type)) = resource_types
                .iter()
                .find(|(candidate, _)| *candidate == resource)
            {
                *runtime_type
            } else {
                let runtime_type = next_runtime_type;
                next_runtime_type = next_runtime_type
                    .checked_add(1)
                    .ok_or_else(|| wasmtime::Error::msg("too many imported resource types"))?;
                resource_types.push((resource, runtime_type));
                definitions.push(ResourceDefinition {
                    interface: Arc::from(interface),
                    name: Arc::from(name.as_str()),
                    runtime_type,
                });
                runtime_type
            };
            let definition = definitions
                .get(runtime_type as usize)
                .cloned()
                .ok_or_else(|| wasmtime::Error::msg("missing host resource definition"))?;
            instance_linker.resource_concurrent(
                &name,
                ResourceType::host_dynamic(runtime_type),
                move |accessor, id| {
                    let definition = definition.clone();
                    Box::pin(async move {
                        let (imports, context, caller) = accessor.with(|mut store| {
                            let data = store.get();
                            let resource = Resource::owned(
                                definition.interface.clone(),
                                definition.name.clone(),
                                id,
                            );
                            data.owned_resources.remove(&resource);
                            (
                                data.imports.clone(),
                                data.context.clone(),
                                data.component.clone(),
                            )
                        });
                        imports
                            .drop_resource(
                                context,
                                caller,
                                Resource::owned(definition.interface, definition.name, id),
                            )
                            .await
                            .map_err(wasmtime::Error::new)
                    })
                },
            )?;
        }
        for (function, concurrent) in functions {
            if concurrent {
                define_concurrent(&mut instance_linker, interface, &function)?;
            } else {
                define_plain(&mut instance_linker, interface, &function)?;
            }
        }
    }
    Ok(definitions)
}

#[derive(Clone)]
pub(crate) struct ResourceDefinition {
    pub(crate) interface: Arc<str>,
    pub(crate) name: Arc<str>,
    pub(crate) runtime_type: u32,
}

fn define_concurrent(
    instance: &mut LinkerInstance<'_, StoreData>,
    interface: &str,
    function: &str,
) -> Result<(), wasmtime::Error> {
    let interface: Arc<str> = Arc::from(interface);
    let function: Arc<str> = Arc::from(function);
    instance.func_new_concurrent(&function.clone(), move |accessor, ty, params, results| {
        let interface = interface.clone();
        let function = function.clone();
        let parameter_types = ty.params().map(|(_, ty)| ty).collect::<Vec<_>>();
        let result_types = ty.results().collect::<Vec<_>>();
        Box::pin(async move {
            let args = convert_params(params, &parameter_types, &mut |value, expected| {
                from_wasmtime(value, expected, &mut |value| match value {
                    LiftValue::Resource(resource) => accessor
                        .with(|store| lift_resource(resource, store))
                        .map(Val::Resource),
                    LiftValue::Future(future) => accessor
                        .with(|mut store| lift_future(future, store.data_mut()))
                        .map(Val::Future),
                    LiftValue::Stream(stream) => accessor
                        .with(|store| lift_stream(stream, store))
                        .map(Val::Stream),
                })
            })?;
            let (imports, context, component) = accessor.with(|mut store| {
                let data = store.get();
                (
                    data.imports.clone(),
                    data.context.clone(),
                    data.component.clone(),
                )
            });
            let values = call(imports, context, component, interface, function, args).await?;
            set_results(results, values, &result_types, &mut |value, expected| {
                to_wasmtime(value, expected, &mut |value| match value {
                    LowerValue::Resource(resource, expected) => accessor
                        .with(|store| lower_resource(&resource, expected, store))
                        .map(WasmtimeVal::Resource),
                    LowerValue::Future(future) => accessor
                        .with(|mut store| lower_future(&future, store.data_mut()))
                        .map(WasmtimeVal::Future),
                    LowerValue::Stream(stream) => accessor
                        .with(|store| lower_stream(stream, store))
                        .map(WasmtimeVal::Stream),
                })
            })
        })
    })
}

fn define_plain(
    instance: &mut LinkerInstance<'_, StoreData>,
    interface: &str,
    function: &str,
) -> Result<(), wasmtime::Error> {
    let interface: Arc<str> = Arc::from(interface);
    let function: Arc<str> = Arc::from(function);
    instance.func_new_async(&function.clone(), move |mut store, ty, params, results| {
        let interface = interface.clone();
        let function = function.clone();
        let parameter_types = ty.params().map(|(_, ty)| ty).collect::<Vec<_>>();
        let result_types = ty.results().collect::<Vec<_>>();
        Box::new(async move {
            let args = convert_params(params, &parameter_types, &mut |value, expected| {
                from_wasmtime(value, expected, &mut |value| match value {
                    LiftValue::Resource(resource) => {
                        lift_resource(resource, store.as_context_mut()).map(Val::Resource)
                    }
                    LiftValue::Future(future) => {
                        lift_future(future, store.data_mut()).map(Val::Future)
                    }
                    LiftValue::Stream(stream) => {
                        lift_stream(stream, store.as_context_mut()).map(Val::Stream)
                    }
                })
            })?;
            let (imports, context, component) = {
                let mut store = store.as_context_mut();
                let data = store.data_mut();
                (
                    data.imports.clone(),
                    data.context.clone(),
                    data.component.clone(),
                )
            };
            let values = call(imports, context, component, interface, function, args).await?;
            set_results(results, values, &result_types, &mut |value, expected| {
                to_wasmtime(value, expected, &mut |value| match value {
                    LowerValue::Resource(resource, expected) => {
                        lower_resource(&resource, expected, store.as_context_mut())
                            .map(WasmtimeVal::Resource)
                    }
                    LowerValue::Future(future) => {
                        lower_future(&future, store.data_mut()).map(WasmtimeVal::Future)
                    }
                    LowerValue::Stream(stream) => {
                        lower_stream(stream, store.as_context_mut()).map(WasmtimeVal::Stream)
                    }
                })
            })
        })
    })
}

fn convert_params(
    params: &[WasmtimeVal],
    parameter_types: &[Type],
    convert: &mut impl FnMut(WasmtimeVal, Option<&Type>) -> Result<Val, wasmtime::Error>,
) -> Result<Vals, wasmtime::Error> {
    params
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, value)| convert(value, parameter_types.get(index)))
        .collect()
}

async fn call(
    imports: Arc<dyn ImportDispatcher>,
    context: InvocationContext,
    component: Arc<str>,
    interface: Arc<str>,
    function: Arc<str>,
    args: Vals,
) -> Result<Vals, wasmtime::Error> {
    imports
        .call(context, component, interface, function, args)
        .await
        .map_err(wasmtime::Error::new)
}

fn set_results(
    results: &mut [WasmtimeVal],
    values: Vals,
    result_types: &[Type],
    convert: &mut impl FnMut(Val, Option<&Type>) -> Result<WasmtimeVal, wasmtime::Error>,
) -> Result<(), wasmtime::Error> {
    if results.len() != values.len() {
        bail!(
            "dispatcher returned {} values for {} result slots",
            values.len(),
            results.len()
        );
    }
    for (index, (result, value)) in results.iter_mut().zip(values).enumerate() {
        *result = convert(value, result_types.get(index))?;
    }
    Ok(())
}
