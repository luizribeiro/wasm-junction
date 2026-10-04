use wasmtime::AsContextMut;
use wasmtime::component::{Linker, Resource, ResourceType};
use wasmtime_wasi::filesystem::WasiFilesystemView;
use wasmtime_wasi::p3::bindings::filesystem::{preopens, types};

use super::{
    CallError, DESCRIPTOR, Descriptor, EngineEvent, FromVal, INTERFACE, JunctionResource,
    PREOPENS_INTERFACE, Real, RealConcurrent, StoreData, ToVal, Val, finish_unit,
    resource_from_val, resource_to_val, scope_values, shape, trampoline, validate_owned_resource,
    views,
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

fn add_drop(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(INTERFACE)?.resource_concurrent(
        DESCRIPTOR,
        ResourceType::host::<Descriptor>(),
        |accessor, id| {
            Box::pin(async move {
                let (invocation, imports, preopen) = accessor.with(|mut access| {
                    let store = access.as_context_mut();
                    let store = store.data();
                    (
                        store.context.invocation_id(),
                        store.imports.clone(),
                        store.descriptor_preopen(id).map(str::to_owned),
                    )
                });
                let invocation = invocation
                    .ok_or_else(|| wasmtime::Error::msg("WASI drop has no invocation id"))?;
                let preopen = preopen
                    .ok_or_else(|| wasmtime::Error::msg("descriptor has no preopen root"))?;
                let resource =
                    JunctionResource::__owned_for_invocation(INTERFACE, DESCRIPTOR, id, invocation);
                imports.emit(EngineEvent::ResourceDrop {
                    invocation,
                    resource: resource.clone(),
                });
                let real: RealConcurrent = |accessor, args| {
                    Box::pin(async move {
                        let [Val::Resource(resource), Val::String(context)] =
                            <[Val; 2]>::try_from(args).map_err(|_| shape(DESCRIPTOR))?
                        else {
                            return Err(shape(DESCRIPTOR));
                        };
                        accessor.with(|mut access| {
                            let mut store = access.as_context_mut();
                            let store = store.data_mut();
                            validate_owned_resource::<Descriptor>(
                                &resource, INTERFACE, DESCRIPTOR, store,
                            )?;
                            if store.descriptor_preopen(resource.id()) != Some(context.as_str()) {
                                return Err(CallError::refused(
                                    "descriptor preopen context does not match",
                                ));
                            }
                            types::HostDescriptor::drop(
                                &mut store.filesystem(),
                                Resource::new_own(resource.id()),
                            )
                            .map_err(|error| CallError::trap(error.to_string()))
                        })?;
                        Ok(Vec::new())
                    })
                };
                let outcome = trampoline::gate_concurrent(
                    accessor,
                    INTERFACE,
                    "[drop]descriptor",
                    vec![Val::Resource(resource), Val::String(preopen)],
                    real,
                )
                .await;
                finish_unit(outcome)?;
                accessor.with(|mut access| {
                    access
                        .as_context_mut()
                        .data_mut()
                        .remove_descriptor_preopen(id);
                });
                Ok(())
            })
        },
    )?;
    Ok(())
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    add_preopens(linker)?;
    add_drop(linker)
}
