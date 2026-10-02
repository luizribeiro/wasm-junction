use std::sync::Arc;

use crate::{BoxFuture, Call, CallContext, CallError, HostBound, Resource, Vals};

/// An object-safe implementation of one host or component interface.
pub trait Provider: HostBound {
    /// Invokes a function through its engine-neutral call representation.
    fn call<'a>(
        &'a self,
        cx: &'a CallContext,
        call: Call,
    ) -> BoxFuture<'a, Result<Vals, CallError>>;

    /// Drops one owned resource after its guest handle is released.
    ///
    /// The hand-written default is unavailable because this base trait stores no resource value.
    /// Generated providers override it to remove the value from their
    /// [`ResourceTable`](crate::ResourceTable) and drop that value by default.
    /// The context identifies the component that released the handle and carries its invocation
    /// data.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] when the resource type or id is unknown.
    fn drop_resource(&self, _cx: &CallContext, resource: Resource) -> Result<(), CallError> {
        Err(CallError::unavailable(format!(
            "provider cannot drop resource `{}/{}` id {}",
            resource.interface(),
            resource.name(),
            resource.id()
        )))
    }
}

/// A provider paired with the fully qualified interface it implements.
pub struct Provided {
    kind: ProvidedKind,
}

pub(crate) enum ProvidedKind {
    Interface {
        interface: &'static str,
        provider: Arc<dyn Provider>,
    },
    Engine(&'static str),
}

impl Provided {
    /// Pairs an implementation with its generated interface identifier for app registration.
    #[must_use]
    pub fn new(interface: &'static str, provider: impl Provider + 'static) -> Self {
        Self {
            kind: ProvidedKind::Interface {
                interface,
                provider: Arc::new(provider),
            },
        }
    }

    pub(crate) const fn engine(name: &'static str) -> Self {
        Self {
            kind: ProvidedKind::Engine(name),
        }
    }

    pub(crate) fn into_kind(self) -> ProvidedKind {
        self.kind
    }
}

impl std::fmt::Debug for Provided {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("Provided");
        match &self.kind {
            ProvidedKind::Interface {
                interface,
                provider,
            } => debug
                .field("interface", interface)
                .field("provider", &Arc::as_ptr(provider)),
            ProvidedKind::Engine(name) => debug.field("engine_provider", name),
        }
        .finish()
    }
}
