use std::error::Error;
use std::fmt::{self, Display};

use crate::{StreamError, TypeError};

/// A failure that crosses a component call boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CallError {
    kind: CallErrorKind,
    message: String,
    trap_code: Option<TrapCode>,
    backtrace: Option<GuestBacktrace>,
    cleanup_error: Option<Box<Self>>,
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

/// A guest WebAssembly backtrace, independent of the engine that captured it.
///
/// ```
/// use wasm_junction_core::{GuestBacktrace, GuestFrame};
///
/// let trace = GuestBacktrace::new(vec![GuestFrame::new(
///     "summarizer",
///     Some("crash".to_owned()),
///     Some(42),
/// )]);
/// assert_eq!(trace.to_string(), "summarizer::crash at wasm offset 0x2a");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestBacktrace {
    frames: Vec<GuestFrame>,
}

impl GuestBacktrace {
    /// Creates a backtrace from frames ordered from innermost to outermost.
    #[must_use]
    pub fn new(frames: Vec<GuestFrame>) -> Self {
        Self { frames }
    }

    /// Returns the captured frames, ordered from innermost to outermost.
    #[must_use]
    pub fn frames(&self) -> &[GuestFrame] {
        &self.frames
    }
}

impl Display for GuestBacktrace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, frame) in self.frames.iter().enumerate() {
            if index != 0 {
                formatter.write_str("\n")?;
            }
            write!(formatter, "{}::", frame.component)?;
            formatter.write_str(frame.function.as_deref().unwrap_or("<unknown>"))?;
            if let Some(offset) = frame.module_offset {
                write!(formatter, " at wasm offset 0x{offset:x}")?;
            }
        }
        Ok(())
    }
}

/// One function frame in a [`GuestBacktrace`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestFrame {
    component: String,
    function: Option<String>,
    module_offset: Option<usize>,
}

impl GuestFrame {
    /// Creates a frame captured while executing `component`.
    #[must_use]
    pub fn new(
        component: impl Into<String>,
        function: Option<String>,
        module_offset: Option<usize>,
    ) -> Self {
        Self {
            component: component.into(),
            function,
            module_offset,
        }
    }

    /// Returns the application component that was executing.
    #[must_use]
    pub fn component(&self) -> &str {
        &self.component
    }

    /// Returns the guest function name when the engine could recover one.
    #[must_use]
    pub fn function(&self) -> Option<&str> {
        self.function.as_deref()
    }

    /// Returns the instruction offset from the start of the WebAssembly module.
    #[must_use]
    pub const fn module_offset(&self) -> Option<usize> {
        self.module_offset
    }
}

impl CallError {
    /// Creates a guest trap with a message suitable for the call's recipient.
    #[must_use]
    pub fn trap(message: impl Into<String>) -> Self {
        Self::new(CallErrorKind::Trap, message)
    }

    /// Creates a trap raised by guest WebAssembly with structured diagnostics.
    ///
    /// Engines use this constructor after translating their own trap and frame
    /// types into [`TrapCode`] and [`GuestBacktrace`].
    #[must_use]
    pub fn guest_trap(
        message: impl Into<String>,
        trap_code: Option<TrapCode>,
        backtrace: Option<GuestBacktrace>,
    ) -> Self {
        Self {
            kind: CallErrorKind::Trap,
            message: message.into(),
            trap_code,
            backtrace,
            cleanup_error: None,
        }
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

    /// Returns the structured cause when guest WebAssembly raised this trap.
    #[must_use]
    pub const fn trap_code(&self) -> Option<TrapCode> {
        self.trap_code
    }

    /// Returns the captured guest frames when this error came from guest code.
    #[must_use]
    pub const fn backtrace(&self) -> Option<&GuestBacktrace> {
        self.backtrace.as_ref()
    }

    /// Returns the newest cleanup failure that happened after this error.
    ///
    /// Earlier cleanup failures remain available by following this accessor
    /// on the returned error.
    #[must_use]
    pub fn cleanup_error(&self) -> Option<&Self> {
        self.cleanup_error.as_deref()
    }

    /// Attaches a cleanup failure without changing this error's kind or message.
    ///
    /// ```
    /// use wasm_junction_core::{CallError, CallErrorKind};
    ///
    /// let error = CallError::refused("private note")
    ///     .with_cleanup_error(CallError::trap("resource drop failed"));
    /// assert_eq!(error.kind(), CallErrorKind::Refused);
    /// assert_eq!(error.to_string(), "private note");
    /// assert_eq!(error.cleanup_error().unwrap().to_string(), "resource drop failed");
    /// ```
    #[must_use]
    pub fn with_cleanup_error(mut self, mut cleanup_error: Self) -> Self {
        if let Some(previous) = self.cleanup_error.take() {
            cleanup_error.append_cleanup_error(*previous);
        }
        self.cleanup_error = Some(Box::new(cleanup_error));
        self
    }

    fn append_cleanup_error(&mut self, cleanup_error: Self) {
        if let Some(existing) = &mut self.cleanup_error {
            existing.append_cleanup_error(cleanup_error);
        } else {
            self.cleanup_error = Some(Box::new(cleanup_error));
        }
    }

    fn new(kind: CallErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            trap_code: None,
            backtrace: None,
            cleanup_error: None,
        }
    }
}

impl Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self.message.lines().next().unwrap_or_default(), formatter)
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
        assert_eq!(call_error.trap_code(), None);
        assert_eq!(call_error.backtrace(), None);
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

    #[test]
    fn guest_backtrace_exposes_and_displays_one_frame_per_line() {
        let backtrace = GuestBacktrace::new(vec![
            GuestFrame::new("summarizer", Some("crash".to_owned()), Some(42)),
            GuestFrame::new("summarizer", None, None),
        ]);

        assert_eq!(backtrace.frames()[0].component(), "summarizer");
        assert_eq!(backtrace.frames()[0].function(), Some("crash"));
        assert_eq!(backtrace.frames()[0].module_offset(), Some(42));
        assert_eq!(
            backtrace.to_string(),
            "summarizer::crash at wasm offset 0x2a\nsummarizer::<unknown>"
        );
    }

    #[test]
    fn call_error_display_is_one_line_and_cleanup_keeps_the_original() {
        let backtrace = GuestBacktrace::new(vec![GuestFrame::new(
            "summarizer",
            Some("crash".to_owned()),
            Some(42),
        )]);
        let error = CallError::guest_trap(
            "wasm trap: unreachable instruction executed\nwasm backtrace follows",
            Some(TrapCode::Unreachable),
            Some(backtrace),
        )
        .with_cleanup_error(CallError::trap("resource cleanup failed"));

        assert_eq!(
            error.to_string(),
            "wasm trap: unreachable instruction executed"
        );
        assert_eq!(error.kind(), CallErrorKind::Trap);
        assert_eq!(error.trap_code(), Some(TrapCode::Unreachable));
        assert_eq!(
            error.backtrace().unwrap().frames()[0].function(),
            Some("crash")
        );
        assert_eq!(
            error.cleanup_error().unwrap().to_string(),
            "resource cleanup failed"
        );
    }

    #[test]
    fn nested_cleanup_failures_are_chained() {
        let error = CallError::refused("private note")
            .with_cleanup_error(CallError::trap("callee cleanup failed"))
            .with_cleanup_error(CallError::trap("caller cleanup failed"));

        assert_eq!(error.kind(), CallErrorKind::Refused);
        assert_eq!(error.to_string(), "private note");
        let caller_cleanup = error.cleanup_error().unwrap();
        assert_eq!(caller_cleanup.to_string(), "caller cleanup failed");
        assert_eq!(
            caller_cleanup.cleanup_error().unwrap().to_string(),
            "callee cleanup failed"
        );
    }

    #[test]
    fn non_guest_errors_have_no_trap_diagnostics() {
        for error in [
            CallError::refused("private note"),
            CallError::unavailable("component unloaded"),
        ] {
            assert_eq!(error.trap_code(), None);
            assert_eq!(error.backtrace(), None);
        }
    }
}
