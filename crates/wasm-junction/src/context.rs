use std::any::Any;
use std::sync::Arc;

use crate::{Caller, Extensions, HostBound, InvocationContext};

/// Information propagated through a call to a host provider.
pub struct CallContext {
    caller: Caller,
    invocation: InvocationContext,
}

impl CallContext {
    pub(crate) fn new(caller: Caller, invocation: InvocationContext) -> Self {
        Self { caller, invocation }
    }

    /// Creates an empty context for directly testing a provider as the named component.
    #[must_use]
    pub fn for_test(name: impl Into<String>) -> Self {
        Self::new(
            Caller::Component(Arc::from(name.into())),
            InvocationContext::default(),
        )
    }

    /// Returns this test context with one attached value.
    #[must_use]
    pub fn with<T: Any + HostBound>(mut self, value: T) -> Self {
        let mut extensions = self.invocation.extensions().clone();
        extensions.insert(value);
        self.invocation = self.invocation.with_extensions(extensions);
        self
    }

    /// Returns the host or named component that made the call.
    #[must_use]
    pub const fn caller(&self) -> &Caller {
        &self.caller
    }

    /// Returns the values attached to this call.
    ///
    /// ```
    /// use wasm_junction::CallContext;
    ///
    /// #[derive(Debug, PartialEq)]
    /// struct SessionId(u64);
    ///
    /// let context = CallContext::for_test("writer").with(SessionId(42));
    /// assert_eq!(context.extensions().get::<SessionId>(), Some(&SessionId(42)));
    /// ```
    #[must_use]
    pub fn extensions(&self) -> &Extensions {
        self.invocation.extensions()
    }

    pub(crate) const fn invocation(&self) -> &InvocationContext {
        &self.invocation
    }
}
