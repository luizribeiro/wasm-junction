use std::any::{Any, TypeId};
use std::collections::HashMap;

use crate::Caller;

#[cfg(not(target_arch = "wasm32"))]
type ExtensionValue = dyn Any + Send + Sync;
#[cfg(target_arch = "wasm32")]
type ExtensionValue = dyn Any;

/// Type-indexed data visible to host providers during one call.
///
/// Middleware and handles will gain APIs for attaching values in a later release. An empty set is
/// already available so provider signatures do not need to change when attachment is added.
#[derive(Default)]
pub struct Extensions {
    values: HashMap<TypeId, Box<ExtensionValue>>,
}

impl Extensions {
    /// Returns the attached value of type `T`, if that type is present.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.values.get(&TypeId::of::<T>())?.downcast_ref()
    }

    /// Returns the attached value of type `T`, if that type is present.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.values.get(&TypeId::of::<T>())?.downcast_ref()
    }
}

/// Information propagated through a call to a host provider.
pub struct CallContext {
    caller: Caller,
    extensions: Extensions,
}

impl CallContext {
    pub(crate) fn new(caller: Caller) -> Self {
        Self {
            caller,
            extensions: Extensions::default(),
        }
    }

    /// Creates an empty context for directly testing a provider as the named component.
    #[must_use]
    pub fn for_test(name: impl Into<String>) -> Self {
        Self::new(Caller::Component(name.into()))
    }

    /// Returns the host or named component that made the call.
    #[must_use]
    pub const fn caller(&self) -> &Caller {
        &self.caller
    }

    /// Returns the values attached to this call.
    #[must_use]
    pub const fn extensions(&self) -> &Extensions {
        &self.extensions
    }
}
