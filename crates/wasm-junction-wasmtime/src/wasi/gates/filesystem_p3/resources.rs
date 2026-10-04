use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::p3::bindings::filesystem::preopens;

use super::{
    CallError, DESCRIPTOR, Descriptor, FromVal, INTERFACE, PREOPENS_INTERFACE, Real, StoreData,
    ToVal, Val, resource_from_val, resource_to_val, scope_values, shape, trampoline, views,
};

fn encode_directories(directories: &[(Resource<Descriptor>, String)]) -> Val {
    Val::List(
        directories
            .iter()
            .map(|(descriptor, path)| {
                Val::Tuple(vec![
                    resource_to_val(descriptor, INTERFACE, DESCRIPTOR),
                    path.clone().to_val(),
                ])
            })
            .collect(),
    )
}

fn decode_directories(value: Val) -> Result<Vec<(Resource<Descriptor>, String)>, CallError> {
    let Val::List(values) = value else {
        return Err(shape("preopen list"));
    };
    values
        .into_iter()
        .map(|value| {
            let Val::Tuple(pair) = value else {
                return Err(shape("preopen"));
            };
            let [descriptor, path] = <[Val; 2]>::try_from(pair).map_err(|_| shape("preopen"))?;
            Ok((
                resource_from_val(descriptor, INTERFACE, DESCRIPTOR)?,
                String::from_val(path)?,
            ))
        })
        .collect()
}

fn add_preopens(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(PREOPENS_INTERFACE)?.func_wrap_async(
        "get-directories",
        |mut store, (): ()| {
            Box::new(async move {
                let real: Real = |mut store, _args| {
                    Box::pin(async move {
                        let directories = preopens::Host::get_directories(&mut views::filesystem(
                            store.data_mut(),
                        ))
                        .map_err(|error| CallError::trap(error.to_string()))?;
                        for (descriptor, path) in &directories {
                            store
                                .data_mut()
                                .set_descriptor_preopen(descriptor.rep(), path.clone());
                        }
                        let invocation = store
                            .data()
                            .context
                            .invocation_id()
                            .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                        Ok(scope_values(
                            vec![encode_directories(&directories)],
                            invocation,
                        ))
                    })
                };
                let outcome = trampoline::gate(
                    &mut store,
                    PREOPENS_INTERFACE,
                    "get-directories",
                    Vec::new(),
                    real,
                )
                .await
                .map_err(wasmtime::Error::new)?;
                let [value] = <[Val; 1]>::try_from(outcome).map_err(|_| shape("preopen list"))?;
                Ok((decode_directories(value)?,))
            })
        },
    )?;
    Ok(())
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    add_preopens(linker)
}
