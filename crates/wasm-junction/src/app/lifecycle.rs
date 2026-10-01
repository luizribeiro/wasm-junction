use std::collections::HashMap;
use std::sync::Arc;

use super::{
    App, Candidate, Generation, LoadedComponent, MissingImports, ReloadError, ResolutionIssue,
    UnloadError, interfaces_compatible, lock_or_recover, resolution_candidates_excluding,
};
use crate::Component;
use crate::component::ComponentParts;

impl App {
    /// Removes a component when no component or live handle depends on it.
    ///
    /// Calls already in progress finish on the removed generation.
    ///
    /// # Errors
    ///
    /// Returns [`UnloadError`] if the component is unknown or has dependents.
    pub async fn unload(&self, name: &str) -> Result<(), UnloadError> {
        // Keep the public lifecycle API async even though generation retirement is immediate.
        std::future::ready(self.unload_inner(name, false)).await
    }

    /// Removes a component even when components or live handles depend on it.
    ///
    /// # Errors
    ///
    /// Returns [`UnloadError`] if the component is unknown.
    pub async fn unload_force(&self, name: &str) -> Result<(), UnloadError> {
        // Keep the public lifecycle API async even though generation retirement is immediate.
        std::future::ready(self.unload_inner(name, true)).await
    }

    fn unload_inner(&self, name: &str, force: bool) -> Result<(), UnloadError> {
        let mut components = self.lock_components();
        if !components.contains_key(name) {
            return Err(UnloadError::UnknownComponent(name.to_owned()));
        }
        if !force {
            let handles = lock_or_recover(&self.0.handle_counts);
            let dependents =
                breaking_dependents(&self.0.providers, &components, &handles, name, &[]);
            if !dependents.is_empty() {
                return Err(UnloadError::HasDependents {
                    component: name.to_owned(),
                    dependents,
                });
            }
        }
        let removed = components
            .remove(name)
            .ok_or_else(|| UnloadError::UnknownComponent(name.to_owned()))?;
        lock_or_recover(&self.0.unloaded)
            .insert(name.to_owned(), removed.generation.exports.clone());
        Ok(())
    }

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
        self.reload_inner(name, component, false).await
    }

    /// Replaces a loaded component even when the change breaks its dependents.
    ///
    /// # Errors
    ///
    /// Returns [`ReloadError`] for structural, resolution, or compilation failures unrelated to
    /// dependents.
    pub async fn reload_force(&self, name: &str, component: Component) -> Result<(), ReloadError> {
        self.reload_inner(name, component, true).await
    }

    async fn reload_inner(
        &self,
        name: &str,
        component: Component,
        force: bool,
    ) -> Result<(), ReloadError> {
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
        self.validate_reload(name, &imports, &exports, force, &self.lock_components())?;
        let compiled = self
            .0
            .engine
            .compile(bytes, self.0.wasi.clone())
            .await
            .map_err(ReloadError::Compile)?;
        let mut components = self.lock_components();
        self.validate_reload(name, &imports, &exports, force, &components)?;
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
        exports: &[String],
        force: bool,
        components: &std::collections::BTreeMap<String, LoadedComponent>,
    ) -> Result<(), ReloadError> {
        let loaded = components
            .get(name)
            .ok_or_else(|| ReloadError::UnknownComponent(name.to_owned()))?;
        let links = retained_links(loaded, imports);
        if !force {
            let handles = lock_or_recover(&self.0.handle_counts);
            let dependents =
                breaking_dependents(&self.0.providers, components, &handles, name, exports);
            if !dependents.is_empty() {
                return Err(ReloadError::Breaking {
                    component: name.to_owned(),
                    dependents,
                });
            }
        }
        let mut missing = Vec::new();
        let mut issues = Vec::new();
        for import in imports {
            if self.0.engine.supports_import(import) {
                continue;
            }
            let candidates = links.get(import.as_str()).map_or_else(
                || {
                    resolution_candidates_excluding(
                        &self.0.providers,
                        components,
                        import,
                        Some(name),
                    )
                },
                |provider| {
                    components
                        .get(provider)
                        .filter(|component| component.exports_interface(import))
                        .map_or_else(Vec::new, |_| vec![Candidate::Component(provider.clone())])
                },
            );
            match candidates.len() {
                0 => missing.push(import.clone()),
                1 => {}
                _ => issues.push(ResolutionIssue::ambiguous(
                    name.to_owned(),
                    import.clone(),
                    candidates,
                )),
            }
        }
        for (consumer, component) in components {
            if consumer == name {
                continue;
            }
            for import in &component.generation.imports {
                if component.links.contains_key(import.as_ref()) {
                    continue;
                }
                let candidates = prospective_candidates(
                    &self.0.providers,
                    components,
                    consumer,
                    import,
                    name,
                    exports,
                );
                if candidates.len() > 1 {
                    issues.push(ResolutionIssue::ambiguous(
                        consumer.clone(),
                        import.to_string(),
                        candidates,
                    ));
                }
            }
        }
        missing.sort();
        missing.dedup();
        issues.sort_by(|left, right| {
            (&left.component, &left.interface).cmp(&(&right.component, &right.interface))
        });
        if !missing.is_empty() {
            return Err(ReloadError::MissingImports(MissingImports::new(missing)));
        }
        if !issues.is_empty() {
            return Err(ReloadError::WouldMakeAmbiguous { issues });
        }
        Ok(())
    }
}

fn breaking_dependents(
    providers: &HashMap<&'static str, Arc<dyn crate::Provider>>,
    components: &std::collections::BTreeMap<String, LoadedComponent>,
    handles: &HashMap<(String, &'static str), usize>,
    name: &str,
    exports: &[String],
) -> Vec<String> {
    let mut dependents = Vec::new();
    for (consumer, component) in components {
        if consumer == name {
            continue;
        }
        for (interface, provider) in &component.links {
            if provider == name
                && !exports
                    .iter()
                    .any(|export| interfaces_compatible(interface, export))
            {
                dependents.push(format!("{consumer} links `{interface}`"));
            }
        }
        for interface in &component.generation.imports {
            if component.links.contains_key(interface.as_ref()) {
                continue;
            }
            let candidates =
                resolution_candidates_excluding(providers, components, interface, Some(consumer));
            if candidates == [Candidate::Component(name.to_owned())]
                && !exports
                    .iter()
                    .any(|export| interfaces_compatible(interface, export))
            {
                dependents.push(format!("{consumer} imports `{interface}`"));
            }
        }
    }
    if let Some(component) = components.get(name) {
        for ((handle_component, interface), count) in handles {
            if handle_component == name
                && *count > 0
                && component.exports_interface(interface)
                && !exports
                    .iter()
                    .any(|export| interfaces_compatible(interface, export))
            {
                dependents.push(format!("host handle for `{name}` uses `{interface}`"));
            }
        }
    }
    dependents.sort();
    dependents.dedup();
    dependents
}

fn prospective_candidates(
    providers: &HashMap<&'static str, Arc<dyn crate::Provider>>,
    components: &std::collections::BTreeMap<String, LoadedComponent>,
    consumer: &str,
    requested: &str,
    replacement: &str,
    exports: &[String],
) -> Vec<Candidate> {
    providers
        .keys()
        .filter(|provided| interfaces_compatible(requested, provided))
        .map(|_| Candidate::Host)
        .chain(components.iter().filter_map(|(name, component)| {
            if name == consumer {
                return None;
            }
            let matches = if name == replacement {
                exports
                    .iter()
                    .any(|export| interfaces_compatible(requested, export))
            } else {
                component.exports_interface(requested)
            };
            matches.then(|| Candidate::Component(name.clone()))
        }))
        .collect()
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
