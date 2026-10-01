use std::sync::Arc;

use super::{App, Generation, ReloadError};
use crate::Component;
use crate::component::ComponentParts;

impl App {
    /// Compiles and atomically replaces a loaded component generation.
    ///
    /// Calls already in progress finish on the generation they started on. New calls and existing
    /// typed handles use the replacement. Explicit links remain when the replacement still imports
    /// a compatible interface; links for removed imports are discarded.
    ///
    /// # Errors
    ///
    /// Returns [`ReloadError`] if the name is not loaded, the replacement exports resources, or
    /// the engine cannot compile it.
    pub async fn reload(&self, name: &str, component: Component) -> Result<(), ReloadError> {
        let ComponentParts {
            bytes,
            imports,
            exports,
            mut resource_exports,
            ..
        } = component.into_parts();
        if !resource_exports.is_empty() {
            resource_exports.sort();
            resource_exports.dedup();
            return Err(ReloadError::ResourceExports(resource_exports));
        }
        if !self.lock_components().contains_key(name) {
            return Err(ReloadError::UnknownComponent(name.to_owned()));
        }
        let compiled = self
            .0
            .engine
            .compile(bytes, self.0.wasi.clone())
            .await
            .map_err(ReloadError::Compile)?;
        let mut components = self.lock_components();
        let loaded = components
            .get_mut(name)
            .ok_or_else(|| ReloadError::UnknownComponent(name.to_owned()))?;
        loaded
            .links
            .retain(|interface, _| imports.iter().any(|import| import == interface));
        loaded.generation = Arc::new(Generation {
            imports: imports.into_iter().map(Arc::from).collect(),
            exports: exports.into_iter().map(Arc::from).collect(),
            compiled,
        });
        Ok(())
    }
}
