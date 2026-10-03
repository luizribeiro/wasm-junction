use wasmtime_wasi::p2::bindings::filesystem::types::{self, Advice, DescriptorType, ErrorCode};

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
