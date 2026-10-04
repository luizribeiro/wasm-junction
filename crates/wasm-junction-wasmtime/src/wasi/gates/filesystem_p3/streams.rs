use wasmtime::AsContextMut;
use wasmtime::component::{Access, FutureReader, Linker, Resource, StreamReader};
use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemView};
use wasmtime_wasi::p3::bindings::filesystem::types::{ErrorCode, HostDescriptorWithStore};

use super::{
    CallError, CallErrorKind, ChannelDirection, DESCRIPTOR, Descriptor, FromVal, INTERFACE,
    RealConcurrent, StoreData, Val, Vals, lift_future_plain, lift_stream_with_direction_plain,
    lower_future_plain, lower_stream_handoff_plain, resource_from_val, resource_to_val,
    scope_values, shape, trampoline,
};
use crate::wasi::gates::filesystem::gate::{add_context_for, validate_context_for};

fn add_context(args: &mut Vals, store: &mut StoreData) -> Result<(), CallError> {
    add_context_for(INTERFACE, args, &[0], store)
}

fn validate_context(args: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_context_for(INTERFACE, args, &[0], store)
}

fn decode_descriptor(value: Val) -> Result<Resource<Descriptor>, CallError> {
    resource_from_val(value, INTERFACE, DESCRIPTOR)
}

pub(super) fn read_real(
    accessor: &wasmtime::component::Accessor<StoreData>,
    args: Vals,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    Box::pin(async move {
        accessor.with(|mut access| {
            let mut store = access.as_context_mut();
            validate_context(&args, store.data_mut())?;
            let mut args = args.into_iter();
            let descriptor = decode_descriptor(args.next().ok_or_else(|| shape(DESCRIPTOR))?)?;
            let offset = u64::from_val(args.next().ok_or_else(|| shape("offset"))?)?;
            let filesystem = Access::<StoreData, WasiFilesystem>::new(
                store.as_context_mut(),
                WasiFilesystemView::filesystem,
            );
            let (stream, future) =
                HostDescriptorWithStore::read_via_stream(filesystem, descriptor, offset)
                    .map_err(|error| CallError::trap(error.to_string()))?;
            Ok(vec![Val::Tuple(vec![
                lift_stream_with_direction_plain(&mut store, stream, ChannelDirection::HostToGuest)
                    .map_err(|error| CallError::trap(error.to_string()))?,
                lift_future_plain(&mut store, future)
                    .map_err(|error| CallError::trap(error.to_string()))?,
            ])])
        })
    })
}

fn add_read(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(INTERFACE)?.func_wrap_concurrent(
        "[method]descriptor.read-via-stream",
        |accessor, (descriptor, offset): (Resource<Descriptor>, u64)| {
            Box::pin(async move {
                let invocation = accessor
                    .with(|mut access| access.get().context.invocation_id())
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(
                    vec![
                        resource_to_val(&descriptor, INTERFACE, DESCRIPTOR),
                        Val::U64(offset),
                    ],
                    invocation,
                );
                accessor
                    .with(|mut access| add_context(&mut args, access.get()))
                    .map_err(wasmtime::Error::new)?;
                let outcome = trampoline::gate_concurrent(
                    accessor,
                    INTERFACE,
                    "[method]descriptor.read-via-stream",
                    args,
                    read_real as RealConcurrent,
                )
                .await;
                let outcome = match outcome {
                    Ok(values) => values,
                    Err(error) if error.kind() == CallErrorKind::Refused => {
                        let (writer, stream) = wasm_junction_core::OutputStream::channel();
                        drop(writer);
                        let (stream, future) =
                            accessor.with(|mut access| -> wasmtime::Result<_> {
                                let mut store = access.as_context_mut();
                                let stream = crate::streams::lower_stream(
                                    wasm_junction_core::StreamHandle::from(stream),
                                    store.as_context_mut(),
                                )?;
                                let future = FutureReader::new(store.as_context_mut(), async {
                                    Ok::<_, wasmtime::Error>(Err(ErrorCode::Access))
                                })?;
                                Ok((stream, future))
                            })?;
                        return Ok(((StreamReader::try_from_stream_any(stream)?, future),));
                    }
                    Err(error) => return Err(wasmtime::Error::new(error)),
                };
                let [Val::Tuple(pair)] =
                    <[Val; 1]>::try_from(outcome).map_err(|_| shape("stream and future"))?
                else {
                    return Err(wasmtime::Error::new(shape("stream and future")));
                };
                let [stream, future] =
                    <[Val; 2]>::try_from(pair).map_err(|_| shape("stream and future"))?;
                accessor.with(|mut access| {
                    let mut store = access.as_context_mut();
                    Ok(((
                        lower_stream_handoff_plain(&mut store, stream)?,
                        lower_future_plain::<Result<(), ErrorCode>>(&mut store, future)?,
                    ),))
                })
            })
        },
    )?;
    Ok(())
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    add_read(linker)
}
