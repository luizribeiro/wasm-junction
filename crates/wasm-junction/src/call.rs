use std::error::Error;
use std::fmt::{self, Display};
use std::sync::Arc;

use crate::{TypeError, Val, Vals};

/// The origin of an invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
    /// A call made directly by the host application.
    Host,
    /// A call made by a loaded component with the given application name.
    Component(Arc<str>),
}

impl Display for Caller {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host => formatter.write_str("host"),
            Self::Component(name) => formatter.write_str(name),
        }
    }
}

/// One engine-neutral interface invocation.
///
/// Middleware uses the routing fields to identify a call and [`Call::view`] when it knows the
/// generated binding for a function. Unknown middleware can inspect or replace [`Call::args`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Call {
    /// The host or component that initiated the call.
    pub caller: Caller,
    /// The application name of the component receiving the call.
    pub callee: Arc<str>,
    /// The fully qualified WIT interface name.
    pub interface: Arc<str>,
    /// The WIT function name.
    pub function: Arc<str>,
    /// The function arguments in declaration order.
    pub args: Vals,
}

impl Call {
    /// Creates a call for dispatch or direct provider testing.
    #[must_use]
    #[expect(
        clippy::similar_names,
        reason = "caller and callee are the routing terms in the public API"
    )]
    pub fn new(
        caller: Caller,
        callee: impl Into<Arc<str>>,
        interface: impl Into<Arc<str>>,
        function: impl Into<Arc<str>>,
        args: Vals,
    ) -> Self {
        Self {
            caller,
            callee: callee.into(),
            interface: interface.into(),
            function: function.into(),
            args,
        }
    }

    /// Decodes this call as `T`, or returns `None` when it names another function.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError`] when the call names `T` but its arguments have the wrong structure.
    pub fn view<T: TypedCall>(&self) -> Result<Option<T>, TypeError> {
        (self.interface.as_ref() == T::INTERFACE && self.function.as_ref() == T::FUNCTION)
            .then(|| T::from_vals(&self.args))
            .transpose()
    }

    /// Replaces this call's arguments with a typed view.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError`] without changing the call when `T` names another function.
    pub fn set_view<T: TypedCall>(&mut self, view: T) -> Result<(), TypeError> {
        if self.interface.as_ref() != T::INTERFACE || self.function.as_ref() != T::FUNCTION {
            return Err(TypeError::new("typed view does not match the call"));
        }
        self.args = view.into_vals();
        Ok(())
    }
}

impl Display for Call {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} → {} {}.{}",
            self.caller, self.callee, self.interface, self.function
        )
    }
}

/// The conversion contract implemented by each generated function view.
pub trait TypedCall: Sized {
    /// The function's typed return value.
    type Output;

    /// The fully qualified WIT interface name.
    const INTERFACE: &'static str;
    /// The WIT function name.
    const FUNCTION: &'static str;

    /// Decodes function arguments from engine-neutral values.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError`] when the arguments have the wrong arity or structure.
    fn from_vals(values: &[Val]) -> Result<Self, TypeError>;

    /// Encodes this typed view as function arguments.
    fn into_vals(self) -> Vals;

    /// Encodes a typed function return value.
    fn output(value: Self::Output) -> Vals;

    /// Decodes a typed function return value.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError`] when the results have the wrong arity or structure.
    fn decode_output(values: &[Val]) -> Result<Self::Output, TypeError>;
}

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
