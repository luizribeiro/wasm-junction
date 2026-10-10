use wasmtime::AsContextMut;
use wasmtime::component::{Access, FutureReader, Linker, Resource, StreamReader};
use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemView};
use wasmtime_wasi::p3::bindings::filesystem::types::{ErrorCode, HostDescriptorWithStore};

use super::{
    CallError, CallErrorKind, ChannelDirection, DESCRIPTOR, Descriptor, FromVal, INTERFACE,
    RealConcurrent, StoreContextMut, StoreData, Val, Vals, deferred, lift_future_plain,
    lift_stream_with_direction_plain, lower_future_plain, lower_stream_handoff_plain,
    resource_from_val, resource_to_val, scope_values, shape, trampoline,
};
use crate::streams::lift_static_stream;
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

fn write_head(
    accessor: &wasmtime::component::Accessor<StoreData>,
    args: Vals,
) -> Result<(Resource<Descriptor>, Val, std::vec::IntoIter<Val>), CallError> {
    accessor.with(|mut access| validate_context(&args, access.as_context_mut().data_mut()))?;
    let mut args = args.into_iter();
    let descriptor = decode_descriptor(args.next().ok_or_else(|| shape(DESCRIPTOR))?)?;
    let stream = args.next().ok_or_else(|| shape("stream"))?;
    Ok((descriptor, stream, args))
}

pub(super) fn write_at_real(
    accessor: &wasmtime::component::Accessor<StoreData>,
    args: Vals,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    Box::pin(async move {
        let (descriptor, stream, mut args) = write_head(accessor, args)?;
        let offset = u64::from_val(args.next().ok_or_else(|| shape("offset"))?)?;
        accessor.with(|mut access| {
            let mut store = access.as_context_mut();
            let stream = lower_stream_handoff_plain(&mut store, stream)
                .map_err(|error| CallError::trap(error.to_string()))?;
            let filesystem = Access::<StoreData, WasiFilesystem>::new(
                store.as_context_mut(),
                WasiFilesystemView::filesystem,
            );
            let future =
                HostDescriptorWithStore::write_via_stream(filesystem, descriptor, stream, offset)
                    .map_err(|error| CallError::trap(error.to_string()))?;
            lift_future_plain(&mut store, future)
                .map(|future| vec![future])
                .map_err(|error| CallError::trap(error.to_string()))
        })
    })
}

pub(super) fn append_real(
    accessor: &wasmtime::component::Accessor<StoreData>,
    args: Vals,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    Box::pin(async move {
        let (descriptor, stream, _) = write_head(accessor, args)?;
        accessor.with(|mut access| {
            let mut store = access.as_context_mut();
            let stream = lower_stream_handoff_plain(&mut store, stream)
                .map_err(|error| CallError::trap(error.to_string()))?;
            let filesystem = Access::<StoreData, WasiFilesystem>::new(
                store.as_context_mut(),
                WasiFilesystemView::filesystem,
            );
            let future = HostDescriptorWithStore::append_via_stream(filesystem, descriptor, stream)
                .map_err(|error| CallError::trap(error.to_string()))?;
            lift_future_plain(&mut store, future)
                .map(|future| vec![future])
                .map_err(|error| CallError::trap(error.to_string()))
        })
    })
}

fn read_body(store: &mut StoreContextMut<'_, StoreData>, args: Vals) -> Result<Vals, CallError> {
    validate_context(&args, store.data_mut())?;
    let mut args = args.into_iter();
    let descriptor = decode_descriptor(args.next().ok_or_else(|| shape(DESCRIPTOR))?)?;
    let offset = u64::from_val(args.next().ok_or_else(|| shape("offset"))?)?;
    let filesystem = Access::<StoreData, WasiFilesystem>::new(
        store.as_context_mut(),
        WasiFilesystemView::filesystem,
    );
    let (stream, future) = HostDescriptorWithStore::read_via_stream(filesystem, descriptor, offset)
        .map_err(|error| CallError::trap(error.to_string()))?;
    Ok(vec![Val::Tuple(vec![
        lift_stream_with_direction_plain(store, stream, ChannelDirection::HostToGuest)
            .map_err(|error| CallError::trap(error.to_string()))?,
        lift_future_plain(store, future).map_err(|error| CallError::trap(error.to_string()))?,
    ])])
}

fn read_directory_body(
    store: &mut StoreContextMut<'_, StoreData>,
    args: Vals,
) -> Result<Vals, CallError> {
    validate_context(&args, store.data_mut())?;
    let descriptor = decode_descriptor(args.into_iter().next().ok_or_else(|| shape(DESCRIPTOR))?)?;
    let filesystem = Access::<StoreData, WasiFilesystem>::new(
        store.as_context_mut(),
        WasiFilesystemView::filesystem,
    );
    let (stream, future) = HostDescriptorWithStore::read_directory(filesystem, descriptor)
        .map_err(|error| CallError::trap(error.to_string()))?;
    Ok(vec![Val::Tuple(vec![
        Val::Stream(
            lift_static_stream(
                stream,
                store.as_context_mut(),
                ChannelDirection::HostToGuest,
            )
            .map_err(|error| CallError::trap(error.to_string()))?,
        ),
        lift_future_plain(store, future).map_err(|error| CallError::trap(error.to_string()))?,
    ])])
}

#[cfg(test)]
fn read_real(
    accessor: &wasmtime::component::Accessor<StoreData>,
    args: Vals,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    Box::pin(
        async move { accessor.with(|mut access| read_body(&mut access.as_context_mut(), args)) },
    )
}

#[cfg(test)]
fn read_directory_real(
    accessor: &wasmtime::component::Accessor<StoreData>,
    args: Vals,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    Box::pin(async move {
        accessor.with(|mut access| read_directory_body(&mut access.as_context_mut(), args))
    })
}

fn read_plain_real(
    mut store: StoreContextMut<'_, StoreData>,
    args: Vals,
) -> wasm_junction_core::BoxFuture<'_, Result<Vals, CallError>> {
    Box::pin(async move { read_body(&mut store, args) })
}

fn add_write(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(INTERFACE)?.func_wrap(
        "[method]descriptor.write-via-stream",
        |mut store, (descriptor, stream, offset): (Resource<Descriptor>, StreamReader<u8>, u64)| {
            let invocation = store
                .data()
                .context
                .invocation_id()
                .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
            let stream = lift_stream_with_direction_plain(
                &mut store,
                stream,
                ChannelDirection::GuestToHost,
            )?;
            let mut args = scope_values(
                vec![
                    resource_to_val(&descriptor, INTERFACE, DESCRIPTOR),
                    stream,
                    Val::U64(offset),
                ],
                invocation,
            );
            add_context(&mut args, store.data_mut()).map_err(wasmtime::Error::new)?;
            let future = deferred::spawn(
                &mut store,
                INTERFACE,
                "[method]descriptor.write-via-stream",
                args,
                write_at_real as RealConcurrent,
                ErrorCode::Access,
            )?;
            Ok((future,))
        },
    )?;
    linker.instance(INTERFACE)?.func_wrap(
        "[method]descriptor.append-via-stream",
        |mut store, (descriptor, stream): (Resource<Descriptor>, StreamReader<u8>)| {
            let invocation = store
                .data()
                .context
                .invocation_id()
                .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
            let stream = lift_stream_with_direction_plain(
                &mut store,
                stream,
                ChannelDirection::GuestToHost,
            )?;
            let mut args = scope_values(
                vec![resource_to_val(&descriptor, INTERFACE, DESCRIPTOR), stream],
                invocation,
            );
            add_context(&mut args, store.data_mut()).map_err(wasmtime::Error::new)?;
            let future = deferred::spawn(
                &mut store,
                INTERFACE,
                "[method]descriptor.append-via-stream",
                args,
                append_real as RealConcurrent,
                ErrorCode::Access,
            )?;
            Ok((future,))
        },
    )?;
    Ok(())
}

fn add_read(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(INTERFACE)?.func_wrap_async(
        "[method]descriptor.read-via-stream",
        |mut store, (descriptor, offset): (Resource<Descriptor>, u64)| {
            Box::new(async move {
                let invocation = store
                    .data()
                    .context
                    .invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(
                    vec![
                        resource_to_val(&descriptor, INTERFACE, DESCRIPTOR),
                        Val::U64(offset),
                    ],
                    invocation,
                );
                add_context(&mut args, store.data_mut()).map_err(wasmtime::Error::new)?;
                let outcome = trampoline::gate(
                    &mut store,
                    INTERFACE,
                    "[method]descriptor.read-via-stream",
                    args,
                    read_plain_real,
                )
                .await;
                let outcome = match outcome {
                    Ok(values) => values,
                    Err(error) if error.kind() == CallErrorKind::Refused => {
                        let (writer, stream) = wasm_junction_core::OutputStream::<u8>::channel();
                        drop(writer);
                        let stream = crate::streams::lower_stream(
                            wasm_junction_core::StreamHandle::from(stream),
                            store.as_context_mut(),
                        )?;
                        let future = FutureReader::new(store.as_context_mut(), async {
                            Ok::<_, wasmtime::Error>(Err(ErrorCode::Access))
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
                Ok(((
                    lower_stream_handoff_plain(&mut store, stream)?,
                    lower_future_plain::<Result<(), ErrorCode>>(&mut store, future)?,
                ),))
            })
        },
    )?;
    Ok(())
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    add_read(linker)?;
    add_write(linker)
}

#[cfg(test)]
mod tests {
    use wasm_junction_core::{InputStream, OutputStream};
    use wasmtime::component::Resource;
    use wasmtime_wasi::FsPerms;
    use wasmtime_wasi::p3::bindings::filesystem::types::{DescriptorFlags, OpenFlags, PathFlags};

    use super::*;
    use crate::wasi::gates::filesystem_p3::descriptors::open_at;
    use crate::wasi::gates::filesystem_p3::test_support::{
        TestDirectory, TestDispatcher, completion, preopens, store,
    };
    use crate::wasi::gates::finish_p3_error;

    async fn open_file(
        accessor: &wasmtime::component::Accessor<StoreData>,
        root: u32,
        path: &str,
        flags: DescriptorFlags,
    ) -> Resource<Descriptor> {
        let filesystem = accessor.with_getter::<WasiFilesystem>(WasiFilesystemView::filesystem);
        open_at(
            &filesystem,
            Resource::new_borrow(root),
            PathFlags::empty(),
            path.to_owned(),
            OpenFlags::empty(),
            flags,
        )
        .await
        .unwrap()
    }

    fn args(
        accessor: &wasmtime::component::Accessor<StoreData>,
        descriptor: u32,
        mut values: Vals,
    ) -> Vals {
        let mut args = vec![resource_to_val(
            &Resource::<Descriptor>::new_borrow(descriptor),
            INTERFACE,
            DESCRIPTOR,
        )];
        args.append(&mut values);
        accessor.with(|mut access| {
            let invocation = access.get().context.invocation_id().unwrap();
            let mut args = scope_values(args, invocation);
            add_context(&mut args, access.get()).unwrap();
            args
        })
    }

    fn one(values: Vals) -> Val {
        let [value] = <[Val; 1]>::try_from(values).unwrap();
        value
    }

    #[tokio::test(flavor = "current_thread")]
    async fn write_and_append_streams_update_the_file_and_complete() {
        let directory = TestDirectory::new("p3-write-append");
        std::fs::write(directory.path().join("note.txt"), b"old").unwrap();
        let mut store = store(
            &[(directory.path(), "/data", FsPerms::ReadWrite)],
            TestDispatcher::passing(),
        );

        store
            .run_concurrent(async |accessor| -> wasmtime::Result<()> {
                let root = preopens(accessor)[0].0;
                let file = open_file(
                    accessor,
                    root,
                    "note.txt",
                    DescriptorFlags::READ | DescriptorFlags::WRITE,
                )
                .await;
                let write = args(
                    accessor,
                    file.rep(),
                    vec![Val::from(OutputStream::from_bytes(b"new")), Val::U64(0)],
                );
                let future = one(write_at_real(accessor, write).await.unwrap());
                assert!(completion(accessor, future).await.is_ok());

                let append = args(
                    accessor,
                    file.rep(),
                    vec![Val::from(OutputStream::from_bytes(b" tail"))],
                );
                let future = one(append_real(accessor, append).await.unwrap());
                assert!(completion(accessor, future).await.is_ok());
                wasmtime::Result::Ok(())
            })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            std::fs::read(directory.path().join("note.txt")).unwrap(),
            b"new tail"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn read_stream_delivers_bytes_and_completes() {
        let directory = TestDirectory::new("p3-read");
        std::fs::write(directory.path().join("note.txt"), b"a useful note").unwrap();
        let mut store = store(
            &[(directory.path(), "/data", FsPerms::ReadWrite)],
            TestDispatcher::passing(),
        );

        store
            .run_concurrent(async |accessor| -> wasmtime::Result<()> {
                let root = preopens(accessor)[0].0;
                let file = open_file(accessor, root, "note.txt", DescriptorFlags::READ).await;
                let read = args(accessor, file.rep(), vec![Val::U64(0)]);
                let [Val::Tuple(pair)] =
                    <[Val; 1]>::try_from(read_real(accessor, read).await.unwrap()).unwrap()
                else {
                    panic!("read returned the wrong shape")
                };
                let [stream, future] = <[Val; 2]>::try_from(pair).unwrap();
                let bytes = InputStream::try_from(stream)
                    .unwrap()
                    .read_all()
                    .await
                    .unwrap();
                assert_eq!(bytes, b"a useful note");
                assert!(completion(accessor, future).await.is_ok());
                wasmtime::Result::Ok(())
            })
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn directory_stream_delivers_named_records_and_completes() {
        let directory = TestDirectory::new("p3-directory");
        std::fs::write(directory.path().join("first.txt"), b"first").unwrap();
        std::fs::create_dir(directory.path().join("notes")).unwrap();
        let mut store = store(
            &[(directory.path(), "/data", FsPerms::ReadWrite)],
            TestDispatcher::passing(),
        );

        store
            .run_concurrent(async |accessor| -> wasmtime::Result<()> {
                let root = preopens(accessor)[0].0;
                let read = args(accessor, root, Vec::new());
                let [Val::Tuple(pair)] =
                    <[Val; 1]>::try_from(read_directory_real(accessor, read).await.unwrap())
                        .unwrap()
                else {
                    panic!("directory read returned the wrong shape")
                };
                let [Val::Stream(stream), future] = <[Val; 2]>::try_from(pair).unwrap() else {
                    panic!("directory read returned the wrong pair")
                };
                let entries = InputStream::<Val>::__from_handle_with(stream, Ok)
                    .unwrap()
                    .read_all()
                    .await
                    .unwrap();
                let names = entries
                    .into_iter()
                    .map(|entry| {
                        let Val::Record(fields) = entry else {
                            panic!("directory entry was not a record")
                        };
                        let [(_, _), (_, Val::String(name))] = <[_; 2]>::try_from(fields).unwrap()
                        else {
                            panic!("directory entry had the wrong fields")
                        };
                        name
                    })
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(names, ["first.txt".to_owned(), "notes".to_owned()].into());
                assert!(completion(accessor, future).await.is_ok());
                wasmtime::Result::Ok(())
            })
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn refused_and_read_only_writes_resolve_as_errors() {
        let directory = TestDirectory::new("p3-refused-write");
        std::fs::write(directory.path().join("note.txt"), b"unchanged").unwrap();
        let mut denied = store(
            &[(directory.path(), "/data", FsPerms::ReadWrite)],
            TestDispatcher::refusing("[method]descriptor.write-via-stream"),
        );
        denied
            .run_concurrent(async |accessor| -> wasmtime::Result<()> {
                let root = preopens(accessor)[0].0;
                let file = open_file(accessor, root, "note.txt", DescriptorFlags::READ).await;
                let write = args(
                    accessor,
                    file.rep(),
                    vec![Val::from(OutputStream::from_bytes(b"denied")), Val::U64(0)],
                );
                let direct = trampoline::gate_concurrent(
                    accessor,
                    INTERFACE,
                    "[method]descriptor.write-via-stream",
                    write,
                    write_at_real,
                )
                .await;
                let direct: Result<(), ErrorCode> =
                    finish_p3_error(direct, ErrorCode::Access, |_| unreachable!()).unwrap();
                assert!(matches!(direct, Err(ErrorCode::Access)));

                let write = args(
                    accessor,
                    file.rep(),
                    vec![Val::from(OutputStream::from_bytes(b"denied")), Val::U64(0)],
                );
                let future = accessor.with(|mut access| {
                    let mut store = access.as_context_mut();
                    let future = deferred::spawn(
                        &mut store,
                        INTERFACE,
                        "[method]descriptor.write-via-stream",
                        write,
                        write_at_real,
                        ErrorCode::Access,
                    )?;
                    lift_future_plain(&mut store, future)
                })?;
                assert!(matches!(
                    completion(accessor, future).await,
                    Err(ErrorCode::Access)
                ));
                wasmtime::Result::Ok(())
            })
            .await
            .unwrap()
            .unwrap();

        let mut read_only = store(
            &[(directory.path(), "/data", FsPerms::ReadOnly)],
            TestDispatcher::passing(),
        );
        read_only
            .run_concurrent(async |accessor| -> wasmtime::Result<()> {
                let root = preopens(accessor)[0].0;
                let file = open_file(accessor, root, "note.txt", DescriptorFlags::READ).await;
                let write = args(
                    accessor,
                    file.rep(),
                    vec![Val::from(OutputStream::from_bytes(b"denied")), Val::U64(0)],
                );
                let future = one(write_at_real(accessor, write).await.unwrap());
                assert!(completion(accessor, future).await.is_err());
                wasmtime::Result::Ok(())
            })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            std::fs::read(directory.path().join("note.txt")).unwrap(),
            b"unchanged"
        );
    }
}
