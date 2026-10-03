use wasmtime::component::Resource;
use wasmtime_wasi::p2::bindings::filesystem::types::{
    self, Advice, DescriptorFlags, DescriptorStat, DescriptorType, DirectoryEntry, ErrorCode,
    HostDescriptor, HostDirectoryEntryStream, MetadataHashValue, NewTimestamp, OpenFlags,
    PathFlags,
};

use super::{FromVal, ToVal, WitResource, open_channel, shape};
use wasm_junction_core::{CallError, ChannelDirection, Val};
use wasmtime::component::Linker;
use wasmtime_wasi::p2::FsResult;
use wasmtime_wasi::p2::bindings::filesystem::preopens;
use wasmtime_wasi::p2::{DynInputStream, DynOutputStream, FsError};

use super::{Real, finish, scope_values, trampoline, views};
use crate::engine::StoreData;

mod gate;

use gate::{
    add_context, add_directory_stream_context, convert, finish_result, gate_fs, validate_context,
    validate_directory_stream_context,
};

const INTERFACE: &str = "wasi:filesystem/types@0.2.12";
const DESCRIPTOR: &str = "descriptor";
const DIRECTORY_ENTRY_STREAM: &str = "directory-entry-stream";

impl WitResource for types::Descriptor {
    const INTERFACE: &'static str = INTERFACE;
    const NAME: &'static str = DESCRIPTOR;
}

impl WitResource for types::DirectoryEntryStream {
    const INTERFACE: &'static str = INTERFACE;
    const NAME: &'static str = DIRECTORY_ENTRY_STREAM;
}

macro_rules! enum_value {
    ($ty:ty { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl ToVal for $ty {
            fn to_val(self) -> Val {
                Val::Enum(match self { $(Self::$variant => $name),+ }.to_owned())
            }
        }

        impl FromVal for $ty {
            fn from_val(value: Val) -> Result<Self, wasm_junction_core::CallError> {
                let Val::Enum(value) = value else { return Err(shape("enum")); };
                match value.as_str() {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(shape(stringify!($ty))),
                }
            }
        }
    };
}

enum_value!(Advice {
    Normal => "normal",
    Sequential => "sequential",
    Random => "random",
    WillNeed => "will-need",
    DontNeed => "dont-need",
    NoReuse => "no-reuse",
});

enum_value!(DescriptorType {
    Unknown => "unknown",
    BlockDevice => "block-device",
    CharacterDevice => "character-device",
    Directory => "directory",
    Fifo => "fifo",
    SymbolicLink => "symbolic-link",
    RegularFile => "regular-file",
    Socket => "socket",
});

enum_value!(ErrorCode {
    Access => "access",
    WouldBlock => "would-block",
    Already => "already",
    BadDescriptor => "bad-descriptor",
    Busy => "busy",
    Deadlock => "deadlock",
    Quota => "quota",
    Exist => "exist",
    FileTooLarge => "file-too-large",
    IllegalByteSequence => "illegal-byte-sequence",
    InProgress => "in-progress",
    Interrupted => "interrupted",
    Invalid => "invalid",
    Io => "io",
    IsDirectory => "is-directory",
    Loop => "loop",
    TooManyLinks => "too-many-links",
    MessageSize => "message-size",
    NameTooLong => "name-too-long",
    NoDevice => "no-device",
    NoEntry => "no-entry",
    NoLock => "no-lock",
    InsufficientMemory => "insufficient-memory",
    InsufficientSpace => "insufficient-space",
    NotDirectory => "not-directory",
    NotEmpty => "not-empty",
    NotRecoverable => "not-recoverable",
    Unsupported => "unsupported",
    NoTty => "no-tty",
    NoSuchDevice => "no-such-device",
    Overflow => "overflow",
    NotPermitted => "not-permitted",
    Pipe => "pipe",
    ReadOnly => "read-only",
    InvalidSeek => "invalid-seek",
    TextFileBusy => "text-file-busy",
    CrossDevice => "cross-device",
});

macro_rules! flags_value {
    ($ty:ty { $($flag:ident => $name:literal),+ $(,)? }) => {
        impl ToVal for $ty {
            fn to_val(self) -> Val {
                Val::Flags(vec![$($name.to_owned(),)+].into_iter().zip([
                    $(self.contains(Self::$flag),)+
                ]).filter_map(|(name, set)| set.then_some(name)).collect())
            }
        }

        impl FromVal for $ty {
            fn from_val(value: Val) -> Result<Self, wasm_junction_core::CallError> {
                let Val::Flags(values) = value else { return Err(shape("flags")); };
                let mut flags = Self::empty();
                for value in values {
                    match value.as_str() {
                        $($name => flags |= Self::$flag,)+
                        _ => return Err(shape(stringify!($ty))),
                    }
                }
                Ok(flags)
            }
        }
    };
}

flags_value!(DescriptorFlags {
    READ => "read",
    WRITE => "write",
    FILE_INTEGRITY_SYNC => "file-integrity-sync",
    DATA_INTEGRITY_SYNC => "data-integrity-sync",
    REQUESTED_WRITE_SYNC => "requested-write-sync",
    MUTATE_DIRECTORY => "mutate-directory",
});
flags_value!(PathFlags { SYMLINK_FOLLOW => "symlink-follow" });
flags_value!(OpenFlags {
    CREATE => "create",
    DIRECTORY => "directory",
    EXCLUSIVE => "exclusive",
    TRUNCATE => "truncate",
});

impl ToVal for NewTimestamp {
    fn to_val(self) -> Val {
        let (case, value) = match self {
            Self::NoChange => ("no-change", None),
            Self::Now => ("now", None),
            Self::Timestamp(value) => ("timestamp", Some(Box::new(value.to_val()))),
        };
        Val::Variant {
            case: case.to_owned(),
            value,
        }
    }
}

impl FromVal for NewTimestamp {
    fn from_val(value: Val) -> Result<Self, wasm_junction_core::CallError> {
        match value {
            Val::Variant { case, value: None } if case == "no-change" => Ok(Self::NoChange),
            Val::Variant { case, value: None } if case == "now" => Ok(Self::Now),
            Val::Variant {
                case,
                value: Some(value),
            } if case == "timestamp" => super::Datetime::from_val(*value).map(Self::Timestamp),
            _ => Err(shape("new-timestamp")),
        }
    }
}

impl ToVal for DescriptorStat {
    fn to_val(self) -> Val {
        Val::Record(vec![
            ("type".to_owned(), self.type_.to_val()),
            ("link-count".to_owned(), self.link_count.to_val()),
            ("size".to_owned(), self.size.to_val()),
            (
                "data-access-timestamp".to_owned(),
                self.data_access_timestamp.to_val(),
            ),
            (
                "data-modification-timestamp".to_owned(),
                self.data_modification_timestamp.to_val(),
            ),
            (
                "status-change-timestamp".to_owned(),
                self.status_change_timestamp.to_val(),
            ),
        ])
    }
}

impl FromVal for DescriptorStat {
    fn from_val(value: Val) -> Result<Self, wasm_junction_core::CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("descriptor-stat"));
        };
        let [
            (_, type_),
            (_, link_count),
            (_, size),
            (_, accessed),
            (_, modified),
            (_, changed),
        ] = <[_; 6]>::try_from(fields).map_err(|_| shape("descriptor-stat fields"))?;
        Ok(Self {
            type_: DescriptorType::from_val(type_)?,
            link_count: u64::from_val(link_count)?,
            size: u64::from_val(size)?,
            data_access_timestamp: Option::<super::Datetime>::from_val(accessed)?,
            data_modification_timestamp: Option::<super::Datetime>::from_val(modified)?,
            status_change_timestamp: Option::<super::Datetime>::from_val(changed)?,
        })
    }
}

impl ToVal for DirectoryEntry {
    fn to_val(self) -> Val {
        Val::Record(vec![
            ("type".to_owned(), self.type_.to_val()),
            ("name".to_owned(), self.name.to_val()),
        ])
    }
}

impl FromVal for DirectoryEntry {
    fn from_val(value: Val) -> Result<Self, wasm_junction_core::CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("directory-entry"));
        };
        let [(_, type_), (_, name)] =
            <[_; 2]>::try_from(fields).map_err(|_| shape("directory-entry fields"))?;
        Ok(Self {
            type_: DescriptorType::from_val(type_)?,
            name: String::from_val(name)?,
        })
    }
}

impl ToVal for MetadataHashValue {
    fn to_val(self) -> Val {
        Val::Record(vec![
            ("lower".to_owned(), self.lower.to_val()),
            ("upper".to_owned(), self.upper.to_val()),
        ])
    }
}

impl FromVal for MetadataHashValue {
    fn from_val(value: Val) -> Result<Self, wasm_junction_core::CallError> {
        let Val::Record(fields) = value else {
            return Err(shape("metadata-hash-value"));
        };
        let [(_, lower), (_, upper)] =
            <[_; 2]>::try_from(fields).map_err(|_| shape("metadata-hash-value fields"))?;
        Ok(Self {
            lower: u64::from_val(lower)?,
            upper: u64::from_val(upper)?,
        })
    }
}

impl<T: ToVal> ToVal for Result<T, ErrorCode> {
    fn to_val(self) -> Val {
        Val::Result(match self {
            Ok(value) => Ok(Some(Box::new(value.to_val()))),
            Err(error) => Err(Some(Box::new(error.to_val()))),
        })
    }
}

impl<T: FromVal> FromVal for Result<T, ErrorCode> {
    fn from_val(value: Val) -> Result<Self, wasm_junction_core::CallError> {
        match value {
            Val::Result(Ok(Some(value))) => T::from_val(*value).map(Ok),
            Val::Result(Ok(None)) => T::from_val(Val::Tuple(Vec::new())).map(Ok),
            Val::Result(Err(Some(error))) => ErrorCode::from_val(*error).map(Err),
            _ => Err(shape("filesystem result")),
        }
    }
}

list_value!((Resource<types::Descriptor>, String));

#[allow(clippy::too_many_arguments)]
async fn open_at(
    store: &mut StoreData,
    descriptor: Resource<types::Descriptor>,
    path_flags: PathFlags,
    path: String,
    open_flags: OpenFlags,
    descriptor_flags: DescriptorFlags,
) -> FsResult<Resource<types::Descriptor>> {
    let preopen = store
        .descriptor_preopen(descriptor.rep())
        .ok_or(ErrorCode::Access)?
        .to_owned();
    let opened = HostDescriptor::open_at(
        &mut views::filesystem(store),
        descriptor,
        path_flags,
        path,
        open_flags,
        descriptor_flags,
    )
    .await?;
    store.set_descriptor_preopen(opened.rep(), preopen);
    Ok(opened)
}

async fn read_directory(
    store: &mut StoreData,
    descriptor: Resource<types::Descriptor>,
) -> FsResult<Resource<types::DirectoryEntryStream>> {
    let preopen = store
        .descriptor_preopen(descriptor.rep())
        .ok_or(ErrorCode::Access)?
        .to_owned();
    let stream = HostDescriptor::read_directory(&mut views::filesystem(store), descriptor).await?;
    store.set_directory_stream_preopen(stream.rep(), preopen);
    Ok(stream)
}

fn read_via_stream(
    store: &mut StoreData,
    descriptor: Resource<types::Descriptor>,
    offset: u64,
) -> FsResult<Resource<DynInputStream>> {
    let stream =
        HostDescriptor::read_via_stream(&mut views::filesystem(store), descriptor, offset)?;
    open_channel(&stream, store, ChannelDirection::HostToGuest).map_err(FsError::trap)?;
    Ok(stream)
}

fn write_via_stream(
    store: &mut StoreData,
    descriptor: Resource<types::Descriptor>,
    offset: u64,
) -> FsResult<Resource<DynOutputStream>> {
    let stream =
        HostDescriptor::write_via_stream(&mut views::filesystem(store), descriptor, offset)?;
    open_channel(&stream, store, ChannelDirection::GuestToHost).map_err(FsError::trap)?;
    Ok(stream)
}

fn append_via_stream(
    store: &mut StoreData,
    descriptor: Resource<types::Descriptor>,
) -> FsResult<Resource<DynOutputStream>> {
    let stream = HostDescriptor::append_via_stream(&mut views::filesystem(store), descriptor)?;
    open_channel(&stream, store, ChannelDirection::GuestToHost).map_err(FsError::trap)?;
    Ok(stream)
}

fn add_directory_entry_stream(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(INTERFACE)?.func_wrap_async(
        "[method]directory-entry-stream.read-directory-entry",
        |mut store, (stream,): (Resource<types::DirectoryEntryStream>,)| {
            Box::new(async move {
                let invocation = store
                    .data()
                    .context
                    .invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let mut args = scope_values(vec![stream.to_val()], invocation);
                add_directory_stream_context(&mut args, store.data_mut())
                    .map_err(wasmtime::Error::new)?;
                let real: Real = |mut store, args| {
                    Box::pin(async move {
                        validate_directory_stream_context(&args, store.data_mut())?;
                        let stream = Resource::<types::DirectoryEntryStream>::from_val(
                            args.into_iter()
                                .next()
                                .ok_or_else(|| shape(DIRECTORY_ENTRY_STREAM))?,
                        )?;
                        let result = HostDirectoryEntryStream::read_directory_entry(
                            &mut views::filesystem(store.data_mut()),
                            stream,
                        )
                        .await;
                        Ok(vec![convert(store.data_mut(), result)?.to_val()])
                    })
                };
                let outcome = trampoline::gate(
                    &mut store,
                    INTERFACE,
                    "[method]directory-entry-stream.read-directory-entry",
                    args,
                    real,
                )
                .await;
                Ok((finish_result::<Option<DirectoryEntry>>(outcome)?,))
            })
        },
    )?;
    Ok(())
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    add_directory_entry_stream(linker)?;
    linker
        .instance("wasi:filesystem/preopens@0.2.12")?
        .func_wrap_async("get-directories", |mut store, (): ()| {
            Box::new(async move {
                let real: Real = |mut store, _args| {
                    Box::pin(async move {
                        let directories = preopens::Host::get_directories(&mut views::filesystem(
                            store.data_mut(),
                        ))
                        .map_err(|error| CallError::trap(error.to_string()))?;
                        for (descriptor, guest_path) in &directories {
                            store
                                .data_mut()
                                .set_descriptor_preopen(descriptor.rep(), guest_path.clone());
                        }
                        let invocation = store
                            .data()
                            .context
                            .invocation_id()
                            .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                        Ok(scope_values(vec![directories.to_val()], invocation))
                    })
                };
                let outcome = trampoline::gate(
                    &mut store,
                    "wasi:filesystem/preopens@0.2.12",
                    "get-directories",
                    Vec::new(),
                    real,
                )
                .await;
                Ok((finish::<Vec<(Resource<types::Descriptor>, String)>>(
                    outcome,
                )?,))
            })
        })?;
    gate_fs!(linker, "[method]descriptor.stat", HostDescriptor::stat, async,
        [0], (descriptor: Resource<types::Descriptor>) -> DescriptorStat);
    gate_fs!(linker, "[method]descriptor.open-at", open_at, store_async,
        [0], (descriptor: Resource<types::Descriptor>, path_flags: PathFlags, path: String,
            open_flags: OpenFlags, descriptor_flags: DescriptorFlags) -> Resource<types::Descriptor>);
    gate_fs!(linker, "[method]descriptor.read-directory", read_directory, store_async,
        [0], (descriptor: Resource<types::Descriptor>) -> Resource<types::DirectoryEntryStream>);
    gate_fs!(linker, "[method]descriptor.read-via-stream", read_via_stream, store_sync,
        [0], (descriptor: Resource<types::Descriptor>, offset: u64) -> Resource<DynInputStream>);
    gate_fs!(linker, "[method]descriptor.write-via-stream", write_via_stream, store_sync,
        [0], (descriptor: Resource<types::Descriptor>, offset: u64) -> Resource<DynOutputStream>);
    gate_fs!(linker, "[method]descriptor.append-via-stream", append_via_stream, store_sync,
        [0], (descriptor: Resource<types::Descriptor>) -> Resource<DynOutputStream>);
    gate_fs!(linker, "[method]descriptor.advise", HostDescriptor::advise, async,
        [0], (descriptor: Resource<types::Descriptor>, offset: u64, len: u64, advice: Advice) -> ());
    gate_fs!(linker, "[method]descriptor.sync-data", HostDescriptor::sync_data, async,
        [0], (descriptor: Resource<types::Descriptor>) -> ());
    gate_fs!(linker, "[method]descriptor.get-flags", HostDescriptor::get_flags, async,
        [0], (descriptor: Resource<types::Descriptor>) -> DescriptorFlags);
    gate_fs!(linker, "[method]descriptor.get-type", HostDescriptor::get_type, async,
        [0], (descriptor: Resource<types::Descriptor>) -> DescriptorType);
    gate_fs!(linker, "[method]descriptor.set-size", HostDescriptor::set_size, async,
        [0], (descriptor: Resource<types::Descriptor>, size: u64) -> ());
    gate_fs!(linker, "[method]descriptor.set-times", HostDescriptor::set_times, async,
        [0], (descriptor: Resource<types::Descriptor>, accessed: NewTimestamp, modified: NewTimestamp) -> ());
    gate_fs!(linker, "[method]descriptor.read", HostDescriptor::read, async,
        [0], (descriptor: Resource<types::Descriptor>, len: u64, offset: u64) -> (Vec<u8>, bool));
    gate_fs!(linker, "[method]descriptor.write", HostDescriptor::write, async,
        [0], (descriptor: Resource<types::Descriptor>, bytes: Vec<u8>, offset: u64) -> u64);
    gate_fs!(linker, "[method]descriptor.sync", HostDescriptor::sync, async,
        [0], (descriptor: Resource<types::Descriptor>) -> ());
    gate_fs!(linker, "[method]descriptor.create-directory-at",
        HostDescriptor::create_directory_at, async,
        [0], (descriptor: Resource<types::Descriptor>, path: String) -> ());
    gate_fs!(linker, "[method]descriptor.stat-at", HostDescriptor::stat_at, async,
        [0], (descriptor: Resource<types::Descriptor>, path_flags: PathFlags, path: String) -> DescriptorStat);
    gate_fs!(linker, "[method]descriptor.set-times-at", HostDescriptor::set_times_at, async,
        [0], (descriptor: Resource<types::Descriptor>, path_flags: PathFlags, path: String,
            accessed: NewTimestamp, modified: NewTimestamp) -> ());
    gate_fs!(linker, "[method]descriptor.link-at", HostDescriptor::link_at, async,
        [0, 3], (descriptor: Resource<types::Descriptor>, old_path_flags: PathFlags,
            old_path: String, new_descriptor: Resource<types::Descriptor>, new_path: String) -> ());
    gate_fs!(linker, "[method]descriptor.readlink-at", HostDescriptor::readlink_at, async,
        [0], (descriptor: Resource<types::Descriptor>, path: String) -> String);
    gate_fs!(linker, "[method]descriptor.remove-directory-at",
        HostDescriptor::remove_directory_at, async,
        [0], (descriptor: Resource<types::Descriptor>, path: String) -> ());
    gate_fs!(linker, "[method]descriptor.rename-at", HostDescriptor::rename_at, async,
        [0, 2], (descriptor: Resource<types::Descriptor>, old_path: String,
            new_descriptor: Resource<types::Descriptor>, new_path: String) -> ());
    gate_fs!(linker, "[method]descriptor.symlink-at", HostDescriptor::symlink_at, async,
        [0], (descriptor: Resource<types::Descriptor>, old_path: String, new_path: String) -> ());
    gate_fs!(linker, "[method]descriptor.unlink-file-at", HostDescriptor::unlink_file_at, async,
        [0], (descriptor: Resource<types::Descriptor>, path: String) -> ());
    gate_fs!(linker, "[method]descriptor.metadata-hash", HostDescriptor::metadata_hash, async,
        [0], (descriptor: Resource<types::Descriptor>) -> MetadataHashValue);
    gate_fs!(linker, "[method]descriptor.metadata-hash-at",
        HostDescriptor::metadata_hash_at, async,
        [0], (descriptor: Resource<types::Descriptor>, path_flags: PathFlags,
            path: String) -> MetadataHashValue);
    Ok(())
}
