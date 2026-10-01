use std::collections::HashMap;
use std::sync::Arc;

use super::{
    App, Generation, LoadedComponent, MissingImports, ReloadError, interfaces_compatible,
    resolution_candidates_excluding,
};
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
        self.validate_reload(name, &imports, &self.lock_components())?;
        let compiled = self
            .0
            .engine
            .compile(bytes, self.0.wasi.clone())
            .await
            .map_err(ReloadError::Compile)?;
        let mut components = self.lock_components();
        self.validate_reload(name, &imports, &components)?;
        let loaded = components
            .get_mut(name)
            .ok_or_else(|| ReloadError::UnknownComponent(name.to_owned()))?;
        loaded.links = retained_links(loaded, &imports);
        loaded.generation = Arc::new(Generation {
            imports: imports.into_iter().map(Arc::from).collect(),
            exports: exports.into_iter().map(Arc::from).collect(),
            compiled,
        });
        Ok(())
    }

    fn validate_reload(
        &self,
        name: &str,
        imports: &[String],
        components: &std::collections::BTreeMap<String, LoadedComponent>,
    ) -> Result<(), ReloadError> {
        let loaded = components
            .get(name)
            .ok_or_else(|| ReloadError::UnknownComponent(name.to_owned()))?;
        let links = retained_links(loaded, imports);
        let mut missing = imports
            .iter()
            .filter(|import| !self.0.engine.supports_import(import))
            .filter(|import| {
                if let Some(provider) = links.get(import.as_str()) {
                    return components
                        .get(provider)
                        .is_none_or(|provider| !provider.exports_interface(import));
                }
                resolution_candidates_excluding(&self.0.providers, components, import, Some(name))
                    .len()
                    != 1
            })
            .cloned()
            .collect::<Vec<_>>();
        missing.sort();
        missing.dedup();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ReloadError::MissingImports(MissingImports::new(missing)))
        }
    }
}

fn retained_links(component: &LoadedComponent, imports: &[String]) -> HashMap<String, String> {
    imports
        .iter()
        .filter_map(|import| {
            component
                .links
                .iter()
                .find(|(linked, _)| interfaces_compatible(import, linked))
                .map(|(_, provider)| (import.clone(), provider.clone()))
        })
        .collect()
}
