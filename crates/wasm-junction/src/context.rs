use std::sync::Arc;

use crate::{Caller, Extensions, InvocationContext};

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

    /// Returns the host or named component that made the call.
    #[must_use]
    pub const fn caller(&self) -> &Caller {
        &self.caller
    }

    /// Returns the values attached to this call.
    #[must_use]
    pub fn extensions(&self) -> &Extensions {
        self.invocation.extensions()
    }
}
