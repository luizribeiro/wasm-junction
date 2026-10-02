use std::collections::HashMap;
use std::sync::Arc;

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;

use crate::{CallError, HostBound, InvocationId};

/// A host resource handle represented independently of any component engine.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Resource {
    interface: Arc<str>,
    name: Arc<str>,
    id: u32,
    ownership: ResourceOwnership,
    invocation: Option<InvocationId>,
}

impl Resource {
    /// Creates a handle for a resource owned by the recipient.
    #[must_use]
    pub fn owned(interface: impl Into<Arc<str>>, name: impl Into<Arc<str>>, id: u32) -> Self {
        Self::new(interface, name, id, ResourceOwnership::Own, None)
    }

    /// Creates a handle borrowed for the duration of a call.
    #[must_use]
    pub fn borrowed(interface: impl Into<Arc<str>>, name: impl Into<Arc<str>>, id: u32) -> Self {
        Self::new(interface, name, id, ResourceOwnership::Borrow, None)
    }

    /// Creates an invocation-owned handle for an engine-provided resource.
    #[doc(hidden)]
    #[must_use]
    pub fn __owned_for_invocation(
        interface: impl Into<Arc<str>>,
        name: impl Into<Arc<str>>,
        id: u32,
        invocation: InvocationId,
    ) -> Self {
        Self::new(
            interface,
            name,
            id,
            ResourceOwnership::Own,
            Some(invocation),
        )
    }

    /// Creates an invocation-borrowed handle for an engine-provided resource.
    #[doc(hidden)]
    #[must_use]
    pub fn __borrowed_for_invocation(
        interface: impl Into<Arc<str>>,
        name: impl Into<Arc<str>>,
        id: u32,
        invocation: InvocationId,
    ) -> Self {
        Self::new(
            interface,
            name,
            id,
            ResourceOwnership::Borrow,
            Some(invocation),
        )
    }

    fn new(
        interface: impl Into<Arc<str>>,
        name: impl Into<Arc<str>>,
        id: u32,
        ownership: ResourceOwnership,
        invocation: Option<InvocationId>,
    ) -> Self {
        Self {
            interface: interface.into(),
            name: name.into(),
            id,
            ownership,
            invocation,
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

    /// Returns the invocation provenance attached by an engine.
    #[doc(hidden)]
    #[must_use]
    pub const fn invocation_id(&self) -> Option<InvocationId> {
        self.invocation
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

/// Validates a resource before an engine lowers it into a component call.
///
/// The returned value tells the engine whether it must retain the resource until ownership is
/// returned or the invocation ends.
#[doc(hidden)]
pub fn validate_resource_lowering(
    resource: &Resource,
    expected_interface: &str,
    expected_name: &str,
    expected_ownership: ResourceOwnership,
) -> Result<bool, CallError> {
    if resource.ownership() != expected_ownership {
        return Err(CallError::refused(format!(
            "resource `{}/{}#{}` has {:?} ownership but the call requires {:?}",
            resource.interface(),
            resource.name(),
            resource.id(),
            resource.ownership(),
            expected_ownership
        )));
    }
    if resource.interface() != expected_interface || resource.name() != expected_name {
        return Err(CallError::refused(format!(
            "resource `{}/{}` does not match the resource type `{expected_interface}/{expected_name}` declared by the call",
            resource.interface(),
            resource.name()
        )));
    }
    Ok(expected_ownership == ResourceOwnership::Own)
}

/// Validates an engine resource's declared type, ownership, and invocation provenance.
#[doc(hidden)]
pub fn validate_resource_for_invocation(
    resource: &Resource,
    expected_interface: &str,
    expected_name: &str,
    expected_ownership: ResourceOwnership,
    invocation: InvocationId,
) -> Result<(), CallError> {
    validate_resource_lowering(
        resource,
        expected_interface,
        expected_name,
        expected_ownership,
    )?;
    if resource.invocation_id() != Some(invocation) {
        return Err(CallError::refused(format!(
            "resource `{expected_interface}/{expected_name}#{}` does not belong to this invocation",
            resource.id()
        )));
    }
    Ok(())
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
    values: HashMap<u32, Arc<T>>,
}

impl<T: HostBound> ResourceTable<T> {
    /// Creates an empty table for one resource from its defining interface.
    #[must_use]
    pub fn new(interface: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self::with_next_id(interface, name, Some(0))
    }

    #[doc(hidden)]
    pub fn __new_with_next_id(
        interface: impl Into<Arc<str>>,
        name: impl Into<Arc<str>>,
        next: u32,
    ) -> Self {
        Self::with_next_id(interface, name, Some(next))
    }

    fn with_next_id(
        interface: impl Into<Arc<str>>,
        name: impl Into<Arc<str>>,
        next: Option<u32>,
    ) -> Self {
        Self {
            interface: interface.into(),
            name: name.into(),
            #[cfg(target_arch = "wasm32")]
            state: RefCell::new(TableState::with_next_id(next)),
            #[cfg(not(target_arch = "wasm32"))]
            state: Mutex::new(TableState::with_next_id(next)),
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
        let id = state.insert(Arc::new(value)).ok_or_else(|| {
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

    /// Borrows the value identified by `resource` while running `operation`.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] for the wrong resource type or an unknown id.
    pub fn with<R>(
        &self,
        resource: &Resource,
        operation: impl FnOnce(&T) -> R,
    ) -> Result<R, CallError> {
        let value = self.borrow(resource)?;
        Ok(operation(&value))
    }

    /// Returns shared ownership of a borrowed value without retaining the table lock.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] for the wrong resource type or an unknown id.
    pub fn borrow(&self, resource: &Resource) -> Result<Arc<T>, CallError> {
        self.validate(resource)?;
        #[cfg(target_arch = "wasm32")]
        let state = self
            .state
            .try_borrow()
            .map_err(|_| CallError::trap("resource table is already borrowed"))?;
        #[cfg(not(target_arch = "wasm32"))]
        let state = self
            .state
            .lock()
            .map_err(|_| CallError::trap("resource table lock is poisoned"))?;
        state
            .values
            .get(&resource.id)
            .cloned()
            .ok_or_else(|| self.unknown(resource.id))
    }

    /// Removes and returns a value whose owned handle crossed back to the provider.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] for a borrow, the wrong resource type, or an unknown id.
    pub fn take(&self, resource: &Resource) -> Result<T, CallError> {
        self.validate(resource)?;
        if resource.ownership != ResourceOwnership::Own {
            return Err(CallError::trap(format!(
                "cannot take borrowed resource `{}/{}`",
                self.interface, self.name
            )));
        }
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
        let value = state
            .values
            .remove(&resource.id)
            .ok_or_else(|| self.unknown(resource.id))?;
        match Arc::try_unwrap(value) {
            Ok(value) => Ok(value),
            Err(value) => {
                state.values.insert(resource.id, value);
                Err(CallError::trap(format!(
                    "cannot take resource `{}/{}` id {} while it is borrowed",
                    self.interface, self.name, resource.id
                )))
            }
        }
    }

    fn validate(&self, resource: &Resource) -> Result<(), CallError> {
        if resource.interface == self.interface && resource.name == self.name {
            Ok(())
        } else {
            Err(CallError::trap(format!(
                "expected resource `{}/{}`, got `{}/{}`",
                self.interface, self.name, resource.interface, resource.name
            )))
        }
    }

    fn unknown(&self, id: u32) -> CallError {
        CallError::trap(format!(
            "unknown resource `{}/{}` id {id}",
            self.interface, self.name
        ))
    }
}

impl<T> TableState<T> {
    fn with_next_id(next: Option<u32>) -> Self {
        Self {
            next,
            values: HashMap::new(),
        }
    }

    fn insert(&mut self, value: Arc<T>) -> Option<u32> {
        let id = self.next?;
        self.next = id.checked_add(1);
        self.values.insert(id, value);
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{TableState, validate_resource_for_invocation, validate_resource_lowering};
    use crate::{InvocationId, Resource, ResourceOwnership, ResourceTable, Val};

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
    fn lowering_validation_reports_whether_ownership_needs_tracking() {
        let owned = Resource::owned("example:notes/store@1.0.0", "note", 7);
        assert!(
            validate_resource_lowering(
                &owned,
                "example:notes/store@1.0.0",
                "note",
                ResourceOwnership::Own,
            )
            .unwrap()
        );
        let error = validate_resource_lowering(
            &owned,
            "example:notes/store@1.0.0",
            "folder",
            ResourceOwnership::Own,
        )
        .unwrap_err();
        assert!(error.to_string().contains("resource type"));
    }

    #[test]
    fn invocation_validation_rejects_foreign_and_unscoped_handles() {
        let current = InvocationId::__from_counter(1);
        let foreign = Resource::__borrowed_for_invocation(
            "wasi:io/poll@0.2.12",
            "pollable",
            3,
            InvocationId::__from_counter(2),
        );
        for resource in [
            foreign,
            Resource::borrowed("wasi:io/poll@0.2.12", "pollable", 3),
        ] {
            let error = validate_resource_for_invocation(
                &resource,
                "wasi:io/poll@0.2.12",
                "pollable",
                ResourceOwnership::Borrow,
                current,
            )
            .unwrap_err();
            assert!(error.to_string().contains("does not belong"));
        }
    }

    #[test]
    fn table_ids_do_not_wrap() {
        let mut state = TableState::with_next_id(Some(u32::MAX));
        assert_eq!(state.insert(Arc::new("last")), Some(u32::MAX));
        assert_eq!(state.insert(Arc::new("overflow")), None);
    }

    #[test]
    fn table_keeps_borrows_and_consumes_owned_values() {
        let table = ResourceTable::new("example:host/api@1.0.0", "session");
        let owned = table.insert(String::from("Ada")).unwrap();
        let borrowed = Resource::borrowed(owned.interface(), owned.name(), owned.id());
        assert_eq!(table.with(&borrowed, String::len).unwrap(), 3);
        assert!(
            table
                .take(&borrowed)
                .unwrap_err()
                .to_string()
                .contains("borrowed")
        );
        assert_eq!(table.take(&owned).unwrap(), "Ada");
        assert!(
            table
                .with(&owned, String::len)
                .unwrap_err()
                .to_string()
                .contains("unknown")
        );
    }

    #[test]
    fn table_rejects_other_resource_types() {
        let table: ResourceTable<()> = ResourceTable::new("example:host/api@1.0.0", "session");
        let wrong = Resource::owned("example:host/api@1.0.0", "file", 0);
        assert!(
            table
                .with(&wrong, |()| ())
                .unwrap_err()
                .to_string()
                .contains("file")
        );
    }

    #[test]
    fn taking_a_borrowed_value_preserves_it_for_a_later_owner() {
        let table = ResourceTable::new("example:host/api@1.0.0", "session");
        let owned = table.insert(String::from("Ada")).unwrap();
        let borrowed = table.borrow(&owned).unwrap();

        let error = table.take(&owned).unwrap_err();
        assert!(error.to_string().contains("while it is borrowed"));
        assert_eq!(borrowed.as_str(), "Ada");

        drop(borrowed);
        assert_eq!(table.take(&owned).unwrap(), "Ada");
    }
}
