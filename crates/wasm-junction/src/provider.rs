use std::sync::Arc;

use crate::{BoxFuture, Call, CallContext, CallError, HostBound, Vals};

/// An object-safe implementation of one host or component interface.
pub trait Provider: HostBound {
    /// Invokes a function through its engine-neutral call representation.
    fn call<'a>(
        &'a self,
        cx: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>>;
}

/// A provider paired with the fully qualified interface it implements.
pub struct Provided {
    interface: &'static str,
    provider: Arc<dyn Provider>,
}

impl Provided {
    /// Pairs an implementation with its generated interface identifier for app registration.
    #[must_use]
    pub fn new(interface: &'static str, provider: impl Provider + 'static) -> Self {
        Self {
            interface,
            provider: Arc::new(provider),
        }
    }

    pub(crate) fn into_parts(self) -> (&'static str, Arc<dyn Provider>) {
        (self.interface, self.provider)
    }
}

impl std::fmt::Debug for Provided {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Provided")
            .field("interface", &self.interface)
            .field("provider", &Arc::as_ptr(&self.provider))
            .finish()
    }
}
