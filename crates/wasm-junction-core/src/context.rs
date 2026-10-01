use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use crate::future::HostBound;

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
    fn insert<T: Any + HostBound>(&mut self, value: T) {
        self.values.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Returns the attached value of type `T`, if that type is present.
    #[must_use]
    pub fn get<T: Any + HostBound>(&self) -> Option<&T> {
        self.values.get(&TypeId::of::<T>())?.downcast_ref()
    }
}

/// Per-invocation data carried through an engine and its imported calls.
#[derive(Clone, Default)]
pub struct InvocationContext {
    extensions: Arc<Extensions>,
    call_depth: usize,
}

impl InvocationContext {
    #[allow(dead_code, reason = "data attachment is not public yet")]
    pub(crate) fn with<T: Any + HostBound>(value: T) -> Self {
        let mut extensions = Extensions::default();
        extensions.insert(value);
        Self {
            extensions: Arc::new(extensions),
            call_depth: 0,
        }
    }

    /// Returns data attached to this invocation.
    #[must_use]
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Returns a copy entered one component-to-component call deeper.
    ///
    /// This is public only for the facade dispatcher across the crate boundary. Engine
    /// implementations should pass invocation context through unchanged.
    #[doc(hidden)]
    #[must_use]
    pub fn descend(&self) -> Self {
        Self {
            extensions: self.extensions.clone(),
            call_depth: self.call_depth.saturating_add(1),
        }
    }

    /// Returns the number of component-to-component calls entered by this invocation.
    ///
    /// This is public only for the facade dispatcher across the crate boundary. Engine
    /// implementations should not interpret this value.
    #[doc(hidden)]
    #[must_use]
    pub const fn call_depth(&self) -> usize {
        self.call_depth
    }
}

#[cfg(test)]
mod tests {
    use super::InvocationContext;

    struct Marker(u32);

    #[test]
    fn attached_data_is_retrieved_by_type() {
        let context = InvocationContext::with(Marker(42));
        let marker = context.extensions().get::<Marker>();

        assert_eq!(marker.map(|marker| marker.0), Some(42));
    }
}
