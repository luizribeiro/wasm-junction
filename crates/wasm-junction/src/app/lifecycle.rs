use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::{
    App, Candidate, Generation, LoadedComponent, MissingImports, ProspectiveComponent, ReloadError,
    UnloadError, interfaces_compatible, lock_or_recover, prospective_components,
    resolution_candidates_excluding, sorted_resource_exports,
};
use crate::Component;
use crate::component::ComponentParts;

struct PendingReload {
    bytes: Arc<[u8]>,
    name: String,
    imports: Vec<String>,
    exports: Vec<String>,
}

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
            let prospective = prospective_components(&components);
            let dependents = breaking_dependents(
                &self.0.providers,
                &components,
                &prospective,
                &handles,
                name,
                &[],
            );
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
        drop(components);
        self.emit(&crate::Event::Unload {
            component: Arc::from(name),
        });
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

    /// Replaces several loaded components as one atomic operation.
    ///
    /// Replacement imports are checked against all other replacements, regardless of order.
    /// Each component keeps explicit links for compatible replacement imports and discards links
    /// for imports it no longer has.
    ///
    /// # Errors
    ///
    /// Returns [`ReloadError`] without changing any component when a target or replacement is
    /// invalid, resolution fails, a change is breaking, or compilation fails.
    pub async fn reload_all<N>(
        &self,
        components: impl IntoIterator<Item = (N, Component)>,
    ) -> Result<(), ReloadError>
    where
        N: Into<String>,
    {
        let mut names = HashSet::new();
        let mut pending = Vec::new();
        for (name, component) in components {
            let name = name.into();
            if !names.insert(name.clone()) {
                return Err(ReloadError::DuplicateTarget(name));
            }
            pending.push(pending_reload(name, component)?);
        }
        self.reload_pending(pending, false).await
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
        let pending = vec![pending_reload(name.to_owned(), component)?];
        self.reload_pending(pending, force).await
    }

    async fn reload_pending(
        &self,
        pending: Vec<PendingReload>,
        force: bool,
    ) -> Result<(), ReloadError> {
        self.validate_reloads(&pending, force, &self.lock_components())?;
        let mut compiled = Vec::with_capacity(pending.len());
        for replacement in &pending {
            compiled.push(
                self.0
                    .engine
                    .compile(replacement.bytes.clone(), self.0.wasi.clone())
                    .await
                    .map_err(ReloadError::Compile)?,
            );
        }
        let mut components = self.lock_components();
        self.validate_reloads(&pending, force, &components)?;
        let mut events = Vec::with_capacity(pending.len());
        for (replacement, compiled) in pending.into_iter().zip(compiled) {
            let loaded = components
                .get_mut(&replacement.name)
                .ok_or_else(|| ReloadError::UnknownComponent(replacement.name.clone()))?;
            loaded.links = retained_links(&loaded.links, &replacement.imports);
            let old_exports = loaded.generation.exports.clone();
            let new_exports = replacement
                .exports
                .iter()
                .cloned()
                .map(Arc::from)
                .collect::<Vec<_>>();
            loaded.generation = Arc::new(Generation {
                imports: replacement.imports.into_iter().map(Arc::from).collect(),
                exports: new_exports.clone(),
                compiled,
            });
            events.push(crate::Event::Reload {
                component: loaded.name.clone(),
                old_exports,
                new_exports,
            });
        }
        drop(components);
        for event in events {
            self.emit(&event);
        }
        Ok(())
    }

    fn validate_reloads(
        &self,
        pending: &[PendingReload],
        force: bool,
        components: &std::collections::BTreeMap<String, LoadedComponent>,
    ) -> Result<(), ReloadError> {
        let handles = lock_or_recover(&self.0.handle_counts);
        let mut prospective = prospective_components(components);
        for replacement in pending {
            let loaded = components
                .get(&replacement.name)
                .ok_or_else(|| ReloadError::UnknownComponent(replacement.name.clone()))?;
            let next = prospective
                .get_mut(&replacement.name)
                .ok_or_else(|| ReloadError::UnknownComponent(replacement.name.clone()))?;
            next.links = retained_links(&loaded.links, &replacement.imports);
            next.imports.clone_from(&replacement.imports);
            next.exports.clone_from(&replacement.exports);
        }
        if !force {
            for replacement in pending {
                let dependents = breaking_dependents(
                    &self.0.providers,
                    components,
                    &prospective,
                    &handles,
                    &replacement.name,
                    &replacement.exports,
                );
                if !dependents.is_empty() {
                    return Err(ReloadError::Breaking {
                        component: replacement.name.clone(),
                        dependents,
                    });
                }
            }
        }
        let required = pending
            .iter()
            .map(|replacement| replacement.name.clone())
            .collect();
        let problems = self.resolve_prospective(&prospective, &required);
        if !problems.missing.is_empty() {
            return Err(ReloadError::MissingImports(MissingImports::new(
                problems.missing,
            )));
        }
        if !problems.ambiguous.is_empty() {
            return Err(ReloadError::WouldMakeAmbiguous {
                issues: problems.ambiguous,
            });
        }
        Ok(())
    }
}

fn pending_reload(name: String, component: Component) -> Result<PendingReload, ReloadError> {
    let ComponentParts {
        bytes,
        imports,
        exports,
        resource_exports,
        ..
    } = component.into_parts();
    let resource_exports = sorted_resource_exports(resource_exports);
    if !resource_exports.is_empty() {
        return Err(ReloadError::ResourceExports(resource_exports));
    }
    Ok(PendingReload {
        bytes,
        name,
        imports,
        exports,
    })
}

fn breaking_dependents(
    providers: &HashMap<&'static str, Arc<dyn crate::Provider>>,
    components: &std::collections::BTreeMap<String, LoadedComponent>,
    consumers: &std::collections::BTreeMap<String, ProspectiveComponent>,
    handles: &HashMap<(String, &'static str), usize>,
    name: &str,
    exports: &[String],
) -> Vec<String> {
    let mut dependents = Vec::new();
    for (consumer, component) in consumers {
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
        for interface in &component.imports {
            if component.links.contains_key(interface) {
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

fn retained_links(links: &HashMap<String, String>, imports: &[String]) -> HashMap<String, String> {
    imports
        .iter()
        .filter_map(|import| {
            links
                .iter()
                .find(|(linked, _)| interfaces_compatible(import, linked))
                .map(|(_, provider)| (import.clone(), provider.clone()))
        })
        .collect()
}
