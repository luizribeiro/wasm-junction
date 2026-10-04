use wasmtime_wasi::p3::bindings::filesystem::types::{
    Advice, DescriptorFlags, DescriptorType, ErrorCode, OpenFlags, PathFlags,
};

use super::{CallError, FromVal, ToVal, Val, shape};

macro_rules! string_variant_value {
    ($ty:ty { $($variant:ident => $name:literal),+; $other:ident => $other_name:literal }) => {
        impl ToVal for $ty {
            fn to_val(self) -> Val {
                let (case, value) = match self {
                    $(Self::$variant => ($name, None),)+
                    Self::$other(value) => ($other_name, Some(Box::new(value.to_val()))),
                };
                Val::Variant { case: case.to_owned(), value }
            }
        }

        impl FromVal for $ty {
            fn from_val(value: Val) -> Result<Self, CallError> {
                let Val::Variant { case, value } = value else {
                    return Err(shape("variant"));
                };
                match (case.as_str(), value) {
                    $(($name, None) => Ok(Self::$variant),)+
                    ($other_name, Some(value)) => Option::<String>::from_val(*value).map(Self::$other),
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

string_variant_value!(DescriptorType {
    BlockDevice => "block-device",
    CharacterDevice => "character-device",
    Directory => "directory",
    Fifo => "fifo",
    SymbolicLink => "symbolic-link",
    RegularFile => "regular-file",
    Socket => "socket";
    Other => "other"
});

string_variant_value!(ErrorCode {
    Access => "access",
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
    CrossDevice => "cross-device";
    Other => "other"
});

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
