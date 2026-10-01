use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

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
    #[cfg(not(target_arch = "wasm32"))]
    fn insert<T: Any + Send + Sync>(&mut self, value: T) {
        self.values.insert(TypeId::of::<T>(), Box::new(value));
    }

    #[cfg(target_arch = "wasm32")]
    fn insert<T: Any>(&mut self, value: T) {
        self.values.insert(TypeId::of::<T>(), Box::new(value));
    }

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

/// Per-invocation data carried through an engine and its imported calls.
#[derive(Clone, Default)]
pub struct InvocationContext(Arc<Extensions>);

impl InvocationContext {
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(dead_code, reason = "data attachment is not public yet")]
    pub(crate) fn with<T: Any + Send + Sync>(value: T) -> Self {
        let mut extensions = Extensions::default();
        extensions.insert(value);
        Self(Arc::new(extensions))
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(dead_code, reason = "data attachment is not public yet")]
    pub(crate) fn with<T: Any>(value: T) -> Self {
        let mut extensions = Extensions::default();
        extensions.insert(value);
        Self(Arc::new(extensions))
    }

    /// Returns data attached to this invocation.
    #[must_use]
    pub fn extensions(&self) -> &Extensions {
        &self.0
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
