use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::future::HostBound;

#[cfg(not(target_arch = "wasm32"))]
type ExtensionValue = dyn Any + Send + Sync;
#[cfg(target_arch = "wasm32")]
type ExtensionValue = dyn Any;

/// Type-indexed data attached to one call.
#[derive(Clone, Default)]
pub struct Extensions {
    values: HashMap<TypeId, Arc<ExtensionValue>>,
}

impl Extensions {
    /// Attaches `value`, replacing the existing value of the same type.
    pub fn insert<T: Any + HostBound>(&mut self, value: T) {
        self.values.insert(TypeId::of::<T>(), Arc::new(value));
    }

    /// Returns the attached value of type `T`, if that type is present.
    #[must_use]
    pub fn get<T: Any + HostBound>(&self) -> Option<&T> {
        self.values.get(&TypeId::of::<T>())?.downcast_ref()
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Extensions")
            .field("len", &self.values.len())
            .finish_non_exhaustive()
    }
}

/// Per-invocation data carried through an engine and its imported calls.
#[derive(Clone, Default)]
pub struct InvocationContext {
    extensions: Extensions,
    settings: Extensions,
    call_depth: usize,
}

impl InvocationContext {
    /// Creates context with one attached value for cross-crate dispatcher tests.
    #[doc(hidden)]
    pub fn with<T: Any + HostBound>(value: T) -> Self {
        let mut extensions = Extensions::default();
        extensions.insert(value);
        Self {
            extensions,
            settings: Extensions::default(),
            call_depth: 0,
        }
    }

    /// Returns data attached to this invocation.
    #[must_use]
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Returns the component settings captured for this invocation.
    #[doc(hidden)]
    #[must_use]
    pub fn settings(&self) -> &Extensions {
        &self.settings
    }

    /// Replaces the attached data while preserving the invocation depth.
    #[doc(hidden)]
    #[must_use]
    pub fn with_extensions(&self, extensions: Extensions) -> Self {
        Self {
            extensions,
            settings: self.settings.clone(),
            call_depth: self.call_depth,
        }
    }

    /// Replaces the component settings while preserving per-call data and invocation depth.
    #[doc(hidden)]
    #[must_use]
    pub fn with_settings(&self, settings: Extensions) -> Self {
        Self {
            extensions: self.extensions.clone(),
            settings,
            call_depth: self.call_depth,
        }
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
            settings: self.settings.clone(),
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
    use std::sync::Arc;

    use super::{Extensions, InvocationContext};

    struct Marker(u32);

    #[test]
    fn attached_data_is_retrieved_by_type() {
        let context = InvocationContext::with(Marker(42));
        let marker = context.extensions().get::<Marker>();

        assert_eq!(marker.map(|marker| marker.0), Some(42));
    }

    #[test]
    fn descending_preserves_attached_data() {
        let context = InvocationContext::with(Marker(42)).descend();

        assert_eq!(context.call_depth(), 1);
        assert_eq!(
            context.extensions().get::<Marker>().map(|marker| marker.0),
            Some(42)
        );
    }

    #[test]
    fn cloned_extensions_share_values_and_replace_by_type() {
        let marker = Arc::new(Marker(42));
        let mut extensions = Extensions::default();
        extensions.insert(marker.clone());

        let clone = extensions.clone();
        assert!(Arc::ptr_eq(clone.get::<Arc<Marker>>().unwrap(), &marker));

        extensions.insert(Arc::new(Marker(7)));
        assert_eq!(extensions.get::<Arc<Marker>>().unwrap().0, 7);
        assert_eq!(clone.get::<Arc<Marker>>().unwrap().0, 42);
    }
}
