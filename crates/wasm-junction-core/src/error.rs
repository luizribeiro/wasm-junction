use std::error::Error;
use std::fmt::{self, Display};

use crate::TypeError;

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
