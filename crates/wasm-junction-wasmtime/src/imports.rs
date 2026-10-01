use std::sync::Arc;

use wasm_junction_core::{CallError, ImportDispatcher, InvocationContext, Vals};
use wasmtime::AsContextMut;
use wasmtime::bail;
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Linker, LinkerInstance, Val as WasmtimeVal};

use crate::engine::StoreData;
use crate::values::{from_wasmtime, to_wasmtime};

pub(crate) fn define_imports(
    linker: &mut Linker<StoreData>,
    component: &Component,
) -> Result<(), wasmtime::Error> {
    let engine = linker.engine().clone();
    for (interface, item) in component.component_type().imports(&engine) {
        let ComponentItem::ComponentInstance(instance) = item.ty else {
            bail!("unsupported component import `{interface}`");
        };
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
        for (function, concurrent) in functions {
            if concurrent {
                define_concurrent(&mut instance_linker, interface, &function)?;
            } else {
                define_plain(&mut instance_linker, interface, &function)?;
            }
        }
    }
    Ok(())
}

fn define_concurrent(
    instance: &mut LinkerInstance<'_, StoreData>,
    interface: &str,
    function: &str,
) -> Result<(), wasmtime::Error> {
    let interface: Arc<str> = Arc::from(interface);
    let function: Arc<str> = Arc::from(function);
    instance.func_new_concurrent(&function.clone(), move |accessor, _, params, results| {
        let interface = interface.clone();
        let function = function.clone();
        Box::pin(async move {
            let args = convert_params(params)?;
            let (imports, context, component) = accessor.with(|mut store| {
                let data = store.get();
                (
                    data.imports.clone(),
                    data.context.clone(),
                    data.component.clone(),
                )
            });
            let values = call(imports, context, component, interface, function, args).await?;
            set_results(results, values)
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
    instance.func_new_async(&function.clone(), move |mut store, _, params, results| {
        let interface = interface.clone();
        let function = function.clone();
        Box::new(async move {
            let args = convert_params(params)?;
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
            set_results(results, values)
        })
    })
}

fn convert_params(params: &[WasmtimeVal]) -> Result<Vals, wasmtime::Error> {
    params.iter().cloned().map(from_wasmtime).collect()
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
        .map_err(|error: CallError| wasmtime::Error::msg(error.to_string()))
}

fn set_results(results: &mut [WasmtimeVal], values: Vals) -> Result<(), wasmtime::Error> {
    if results.len() != values.len() {
        bail!(
            "dispatcher returned {} values for {} result slots",
            values.len(),
            results.len()
        );
    }
    for (result, value) in results.iter_mut().zip(values) {
        *result = to_wasmtime(value)?;
    }
    Ok(())
}
