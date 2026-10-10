#[allow(
    clippy::wildcard_imports,
    reason = "filesystem gates share the parent module's private gate machinery"
)]
use super::*;
use wasmtime_wasi::filesystem::Descriptor;

pub(super) const INTERFACE: &str = "wasi:filesystem/types@0.3.0";
const PREOPENS_INTERFACE: &str = "wasi:filesystem/preopens@0.3.0";
const DESCRIPTOR: &str = "descriptor";

mod descriptors;
mod records;
mod resources;
mod streams;
#[cfg(test)]
mod test_support;
mod values;

pub(super) fn add(
    linker: &mut wasmtime::component::Linker<crate::engine::StoreData>,
) -> wasmtime::Result<()> {
    resources::add(linker)?;
    descriptors::add_metadata(linker)?;
    descriptors::add_paths(linker)?;
    descriptors::add_identity(linker)?;
    streams::add(linker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_gates_register() {
        let mut config = wasmtime::Config::new();
        config
            .wasm_component_model_async(true)
            .concurrency_support(true);
        let engine = wasmtime::Engine::new(&config).unwrap();
        let mut linker = wasmtime::component::Linker::new(&engine);
        add(&mut linker).unwrap();
    }
}
