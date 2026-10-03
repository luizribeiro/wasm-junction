use crate::InvocationId;

/// An opaque handle to a WIT `future<T>` owned by one component invocation.
///
/// The payload type and completion are managed by the component engine. Middleware can retain,
/// compare, and forward the handle within the call that produced it, but cannot await it directly.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FutureHandle {
    id: u64,
    invocation: InvocationId,
}

impl FutureHandle {
    /// Creates a handle for an engine-owned future in an invocation.
    #[doc(hidden)]
    #[must_use]
    pub const fn __for_invocation(id: u64, invocation: InvocationId) -> Self {
        Self { id, invocation }
    }

    /// Returns the framework identifier for this future.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Returns the invocation that owns this future.
    #[must_use]
    pub const fn invocation_id(&self) -> InvocationId {
        self.invocation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_exposes_opaque_identity_and_scope() {
        let invocation = InvocationId::__from_counter(7);
        let handle = FutureHandle::__for_invocation(11, invocation);
        assert_eq!(handle.id(), 11);
        assert_eq!(handle.invocation_id(), invocation);
        assert_eq!(handle, handle.clone());
    }
}
