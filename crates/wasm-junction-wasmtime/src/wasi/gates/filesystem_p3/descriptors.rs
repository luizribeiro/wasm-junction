use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemView};
use wasmtime_wasi::p3::bindings::filesystem::types::{
    Advice, DescriptorFlags, DescriptorStat, DescriptorType, ErrorCode, HostDescriptorWithStore,
    MetadataHashValue, NewTimestamp, OpenFlags, PathFlags,
};

use super::{
    CallError, DESCRIPTOR, Descriptor, FromVal, INTERFACE, RealConcurrent, ToVal, Val, Vals,
    convert_trappable, finish_p3_error, resource_from_val, resource_to_val, scope_values, shape,
    trampoline,
};
use crate::engine::StoreData;
use crate::wasi::gates::filesystem::gate::{add_context_for, validate_context_for};

fn add_context(
    args: &mut Vals,
    positions: &[usize],
    store: &mut StoreData,
) -> Result<(), CallError> {
    add_context_for(INTERFACE, args, positions, store)
}

fn validate_context(
    args: &[Val],
    positions: &[usize],
    store: &mut StoreData,
) -> Result<(), CallError> {
    validate_context_for(INTERFACE, args, positions, store)
}

trait P3Value: Sized {
    fn to_p3_val(self) -> Val;
    fn from_p3_val(value: Val) -> Result<Self, CallError>;
}

macro_rules! p3_value_via_traits {
    ($($ty:ty),+ $(,)?) => {$(
        impl P3Value for $ty {
            fn to_p3_val(self) -> Val { self.to_val() }
            fn from_p3_val(value: Val) -> Result<Self, CallError> { Self::from_val(value) }
        }
    )+};
}

p3_value_via_traits!(
    (),
    u64,
    String,
    Advice,
    DescriptorFlags,
    DescriptorStat,
    DescriptorType,
    ErrorCode,
    MetadataHashValue,
    NewTimestamp,
    OpenFlags,
    PathFlags,
);

impl P3Value for Resource<Descriptor> {
    fn to_p3_val(self) -> Val {
        resource_to_val(&self, INTERFACE, DESCRIPTOR)
    }

    fn from_p3_val(value: Val) -> Result<Self, CallError> {
        resource_from_val(value, INTERFACE, DESCRIPTOR)
    }
}

fn result_value<T: P3Value>(result: Result<T, ErrorCode>) -> Val {
    Val::Result(match result {
        Ok(value) => Ok(Some(Box::new(value.to_p3_val()))),
        Err(error) => Err(Some(Box::new(error.to_p3_val()))),
    })
}

fn decode_result<T: P3Value>(values: Vals) -> Result<Result<T, ErrorCode>, CallError> {
    let [Val::Result(result)] = <[Val; 1]>::try_from(values).map_err(|_| shape("result"))? else {
        return Err(shape("result"));
    };
    match result {
        Ok(value) => {
            T::from_p3_val(value.map_or_else(|| Val::Tuple(Vec::new()), |value| *value)).map(Ok)
        }
        Err(value) => {
            ErrorCode::from_p3_val(value.map_or_else(|| Val::Tuple(Vec::new()), |value| *value))
                .map(Err)
        }
    }
}

macro_rules! gate_descriptor {
    ($linker:ident, $name:literal, $method:path, [$($position:expr),+],
     ($($arg:ident: $ty:ty),*) -> $ok:ty) => {
        $linker.instance(INTERFACE)?.func_wrap_concurrent(
            $name,
            |accessor, ($($arg,)*): ($($ty,)*)| Box::pin(async move {
                let invocation = accessor.with(|mut access| access.get().context.invocation_id())
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(vec![$($arg.to_p3_val()),*], invocation);
                accessor.with(|mut access| add_context(&mut args, &[$($position),+], access.get()))
                    .map_err(wasmtime::Error::new)?;
                let real: RealConcurrent = |accessor, args| Box::pin(async move {
                    accessor.with(|mut access| validate_context(
                        &args, &[$($position),+], access.get(),
                    ))?;
                    let mut args = args.into_iter();
                    $(let $arg = <$ty>::from_p3_val(args.next()
                        .ok_or_else(|| shape("another argument"))?)?;)*
                    let view = accessor.with_getter::<WasiFilesystem>(WasiFilesystemView::filesystem);
                    let result = $method(&view $(, $arg)*).await;
                    Ok(vec![result_value(convert_trappable(result)?)])
                });
                let outcome = trampoline::gate_concurrent(
                    accessor, INTERFACE, $name, args, real,
                ).await;
                Ok((finish_p3_error(
                    outcome, ErrorCode::Access, decode_result::<$ok>,
                )?,))
            }),
        )?;
    };
}

pub(super) fn add_metadata(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_descriptor!(linker, "[method]descriptor.advise", HostDescriptorWithStore::advise, [0],
        (descriptor: Resource<Descriptor>, offset: u64, length: u64, advice: Advice) -> ());
    gate_descriptor!(linker, "[method]descriptor.sync-data", HostDescriptorWithStore::sync_data,
        [0], (descriptor: Resource<Descriptor>) -> ());
    gate_descriptor!(linker, "[method]descriptor.get-flags", HostDescriptorWithStore::get_flags,
        [0], (descriptor: Resource<Descriptor>) -> DescriptorFlags);
    gate_descriptor!(linker, "[method]descriptor.get-type", HostDescriptorWithStore::get_type,
        [0], (descriptor: Resource<Descriptor>) -> DescriptorType);
    gate_descriptor!(linker, "[method]descriptor.set-size", HostDescriptorWithStore::set_size,
        [0], (descriptor: Resource<Descriptor>, size: u64) -> ());
    gate_descriptor!(linker, "[method]descriptor.set-times", HostDescriptorWithStore::set_times,
        [0], (descriptor: Resource<Descriptor>, accessed: NewTimestamp,
            modified: NewTimestamp) -> ());
    gate_descriptor!(linker, "[method]descriptor.stat", HostDescriptorWithStore::stat,
        [0], (descriptor: Resource<Descriptor>) -> DescriptorStat);
    gate_descriptor!(linker, "[method]descriptor.sync", HostDescriptorWithStore::sync,
        [0], (descriptor: Resource<Descriptor>) -> ());
    gate_descriptor!(linker, "[method]descriptor.metadata-hash",
        HostDescriptorWithStore::metadata_hash, [0],
        (descriptor: Resource<Descriptor>) -> MetadataHashValue);
    gate_descriptor!(linker, "[method]descriptor.metadata-hash-at",
        HostDescriptorWithStore::metadata_hash_at, [0],
        (descriptor: Resource<Descriptor>, path_flags: PathFlags,
            path: String) -> MetadataHashValue);
    Ok(())
}
