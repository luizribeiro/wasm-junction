use wasmtime_wasi::p2::bindings::filesystem::types::{
    self, Advice, DescriptorFlags, DescriptorStat, DescriptorType, DirectoryEntry, ErrorCode,
    MetadataHashValue, NewTimestamp, OpenFlags, PathFlags,
};

use super::{FromVal, ToVal, WitResource, shape};
use wasm_junction_core::Val;

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
