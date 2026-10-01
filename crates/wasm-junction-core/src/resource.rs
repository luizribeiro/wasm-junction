use std::sync::Arc;

/// A host resource handle represented independently of any component engine.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Resource {
    interface: Arc<str>,
    name: Arc<str>,
    id: u32,
    ownership: ResourceOwnership,
}

impl Resource {
    /// Creates a handle for a resource owned by the recipient.
    #[must_use]
    pub fn owned(interface: impl Into<Arc<str>>, name: impl Into<Arc<str>>, id: u32) -> Self {
        Self::new(interface, name, id, ResourceOwnership::Own)
    }

    /// Creates a handle borrowed for the duration of a call.
    #[must_use]
    pub fn borrowed(interface: impl Into<Arc<str>>, name: impl Into<Arc<str>>, id: u32) -> Self {
        Self::new(interface, name, id, ResourceOwnership::Borrow)
    }

    fn new(
        interface: impl Into<Arc<str>>,
        name: impl Into<Arc<str>>,
        id: u32,
        ownership: ResourceOwnership,
    ) -> Self {
        Self {
            interface: interface.into(),
            name: name.into(),
            id,
            ownership,
        }
    }

    /// Returns the defining versioned interface name.
    #[must_use]
    pub fn interface(&self) -> &str {
        &self.interface
    }

    /// Returns the resource name within its defining interface.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the provider-assigned identifier.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Returns whether ownership moves with this handle.
    #[must_use]
    pub const fn ownership(&self) -> ResourceOwnership {
        self.ownership
    }
}

/// Whether a [`Resource`] transfers ownership or temporarily borrows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceOwnership {
    /// Ownership moves to the recipient.
    Own,
    /// The recipient borrows the resource for one call.
    Borrow,
}

#[cfg(test)]
mod tests {
    use crate::{Resource, ResourceOwnership, Val};

    #[test]
    fn resource_value_preserves_identity_and_ownership() {
        let resource = Resource::borrowed("example:host/files@1.0.0", "file", 7);
        assert_eq!(resource.interface(), "example:host/files@1.0.0");
        assert_eq!(resource.name(), "file");
        assert_eq!(resource.id(), 7);
        assert_eq!(resource.ownership(), ResourceOwnership::Borrow);
        assert_eq!(Val::Resource(resource.clone()), Val::Resource(resource));
    }
}
