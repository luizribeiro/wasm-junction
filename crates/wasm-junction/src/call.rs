use std::error::Error;
use std::fmt::{self, Display};

use crate::{TypeError, Val, Vals};

/// The origin of an invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
    /// A call made directly by the host application.
    Host,
    /// A call made by a loaded component with the given application name.
    Component(String),
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
/// Middleware uses the routing fields to identify a call and can inspect or replace [`Call::args`]
/// when it does not know the generated binding for a function.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// The host or component that initiated the call.
    pub caller: Caller,
    /// The application name of the component receiving the call.
    pub callee: String,
    /// The fully qualified WIT interface name.
    pub interface: &'static str,
    /// The WIT function name.
    pub function: &'static str,
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
        callee: impl Into<String>,
        interface: &'static str,
        function: &'static str,
        args: Vals,
    ) -> Self {
        Self {
            caller,
            callee: callee.into(),
            interface,
            function,
            args,
        }
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
pub struct Trap(String);

impl Trap {
    /// Creates a trap with a message suitable for the call's recipient.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl Display for Trap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}

impl Error for Trap {}

impl From<TypeError> for Trap {
    fn from(error: TypeError) -> Self {
        Self(error.to_string())
    }
}
