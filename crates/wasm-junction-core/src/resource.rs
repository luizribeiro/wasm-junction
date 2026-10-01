use std::collections::HashMap;
use std::sync::Arc;

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;

use crate::{CallError, HostBound};

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

/// A provider-owned table that keeps host resource values alive across calls.
pub struct ResourceTable<T: HostBound> {
    interface: Arc<str>,
    name: Arc<str>,
    #[cfg(target_arch = "wasm32")]
    state: RefCell<TableState<T>>,
    #[cfg(not(target_arch = "wasm32"))]
    state: Mutex<TableState<T>>,
}

struct TableState<T> {
    next: Option<u32>,
    values: HashMap<u32, T>,
}

impl<T: HostBound> ResourceTable<T> {
    /// Creates an empty table for one resource from its defining interface.
    #[must_use]
    pub fn new(interface: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            interface: interface.into(),
            name: name.into(),
            #[cfg(target_arch = "wasm32")]
            state: RefCell::new(TableState::new()),
            #[cfg(not(target_arch = "wasm32"))]
            state: Mutex::new(TableState::new()),
        }
    }

    /// Stores a value and returns a newly owned handle whose id is never reused.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] after all `u32` ids have been allocated.
    pub fn insert(&self, value: T) -> Result<Resource, CallError> {
        #[cfg(target_arch = "wasm32")]
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| CallError::trap("resource table is already borrowed"))?;
        #[cfg(not(target_arch = "wasm32"))]
        let mut state = self
            .state
            .lock()
            .map_err(|_| CallError::trap("resource table lock is poisoned"))?;
        let id = state.insert(value).ok_or_else(|| {
            CallError::trap(format!(
                "resource table for `{}/{}` is exhausted",
                self.interface, self.name
            ))
        })?;
        Ok(Resource::owned(
            self.interface.clone(),
            self.name.clone(),
            id,
        ))
    }
}

impl<T> TableState<T> {
    fn new() -> Self {
        Self {
            next: Some(0),
            values: HashMap::new(),
        }
    }

    fn insert(&mut self, value: T) -> Option<u32> {
        let id = self.next?;
        self.next = id.checked_add(1);
        self.values.insert(id, value);
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::TableState;
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

    #[test]
    fn table_ids_do_not_wrap() {
        let mut state = TableState::new();
        state.next = Some(u32::MAX);
        assert_eq!(state.insert("last"), Some(u32::MAX));
        assert_eq!(state.insert("overflow"), None);
    }
}
