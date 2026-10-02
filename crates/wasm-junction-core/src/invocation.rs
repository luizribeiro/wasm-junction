use std::fmt::{self, Display};

/// An opaque identifier unique to one component invocation within an application.
///
/// Identifiers are stable for the invocation's lifetime and may be used as middleware map keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InvocationId(u64);

impl InvocationId {
    /// The value held by a call before application dispatch assigns its identifier.
    #[doc(hidden)]
    pub const __UNASSIGNED: Self = Self(0);

    /// Returns the first value available to an application's invocation counter.
    #[doc(hidden)]
    #[must_use]
    pub const fn __first_counter() -> u64 {
        Self::__UNASSIGNED.0 + 1
    }

    /// Creates an identifier from an application-local counter.
    #[doc(hidden)]
    #[must_use]
    pub const fn __from_counter(value: u64) -> Self {
        Self(value)
    }
}

impl Display for InvocationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}
