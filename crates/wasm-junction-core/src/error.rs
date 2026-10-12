use std::error::Error;
use std::fmt::{self, Display};

use crate::{StreamError, TypeError};

/// A failure that crosses a component call boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CallError {
    kind: CallErrorKind,
    message: String,
}

/// The broad category of a [`CallError`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallErrorKind {
    /// The guest trapped or the call otherwise failed during execution.
    Trap,
    /// Middleware or a provider refused the call.
    Refused,
    /// The requested component, export, or provider is unavailable.
    Unavailable,
}

/// A structured reason for a trap raised by guest WebAssembly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrapCode {
    /// The guest exhausted its call stack.
    StackOverflow,
    /// The guest accessed memory outside its bounds.
    MemoryOutOfBounds,
    /// An atomic memory access was not naturally aligned.
    HeapMisaligned,
    /// The guest accessed a table outside its bounds.
    TableOutOfBounds,
    /// An indirect call targeted a null table entry.
    IndirectCallToNull,
    /// An indirect call used a function with the wrong type.
    IndirectCallTypeMismatch,
    /// Integer arithmetic overflowed.
    IntegerOverflow,
    /// Integer division used a zero divisor.
    IntegerDivideByZero,
    /// A floating-point value could not be converted to an integer.
    BadConversionToInteger,
    /// The guest executed an unreachable instruction.
    Unreachable,
    /// The host interrupted guest execution.
    Interrupt,
    /// The guest exhausted its execution fuel.
    OutOfFuel,
    /// The guest dereferenced a null reference.
    NullReference,
    /// The guest accessed an array outside its bounds.
    ArrayOutOfBounds,
    /// A guest allocation was too large.
    AllocationTooLarge,
    /// A reference cast failed.
    CastFailure,
}

impl Display for TrapCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::StackOverflow => "call stack exhausted",
            Self::MemoryOutOfBounds => "out of bounds memory access",
            Self::HeapMisaligned => "unaligned atomic memory access",
            Self::TableOutOfBounds => "out of bounds table access",
            Self::IndirectCallToNull => "indirect call to a null table entry",
            Self::IndirectCallTypeMismatch => "indirect call type mismatch",
            Self::IntegerOverflow => "integer overflow",
            Self::IntegerDivideByZero => "integer divide by zero",
            Self::BadConversionToInteger => "invalid conversion to integer",
            Self::Unreachable => "unreachable instruction executed",
            Self::Interrupt => "execution interrupted",
            Self::OutOfFuel => "execution fuel exhausted",
            Self::NullReference => "null reference",
            Self::ArrayOutOfBounds => "out of bounds array access",
            Self::AllocationTooLarge => "allocation too large",
            Self::CastFailure => "reference cast failed",
        };
        formatter.write_str(reason)
    }
}
impl CallError {
    /// Creates a guest trap with a message suitable for the call's recipient.
    #[must_use]
    pub fn trap(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::Trap, message)
    }

    /// Creates a refusal with a message suitable for the call's recipient.
    #[must_use]
    pub fn refused(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::Refused, message)
    }

    /// Creates an unavailable-target error with a message suitable for the caller.
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::Unavailable, message)
    }

    /// Returns the broad category of this error.
    #[must_use]
    pub const fn kind(&self) -> CallErrorKind {
        self.kind
    }

    fn new(kind: CallErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.message, formatter)
    }
}

impl Error for CallError {}

impl From<TypeError> for CallError {
    fn from(error: TypeError) -> Self {
        Self::trap(error.to_string())
    }
}

impl From<StreamError> for CallError {
    fn from(error: StreamError) -> Self {
        Self::trap(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::{InputStream, OutputStream, Val};

    use super::*;

    #[test]
    fn stream_errors_become_call_traps() {
        let value = Val::from(OutputStream::from_bytes(b"contents"));
        let _reader = InputStream::try_from(value.clone()).unwrap();
        let Err(stream_error) = InputStream::try_from(value) else {
            panic!("stream unexpectedly gained another reader");
        };

        let call_error = CallError::from(stream_error);
        assert_eq!(call_error.kind(), CallErrorKind::Trap);
        assert_eq!(call_error.to_string(), "stream already has a reader");
    }

    #[test]
    fn trap_codes_have_engine_neutral_messages() {
        assert_eq!(
            TrapCode::Unreachable.to_string(),
            "unreachable instruction executed"
        );
        assert_eq!(
            TrapCode::MemoryOutOfBounds.to_string(),
            "out of bounds memory access"
        );
    }
}
