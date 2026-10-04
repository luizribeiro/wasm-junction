#[allow(
    clippy::wildcard_imports,
    reason = "filesystem gates share the parent module's private gate machinery"
)]
use super::*;
use wasmtime_wasi::filesystem::Descriptor;

const INTERFACE: &str = "wasi:filesystem/types@0.3.0";
#[allow(
    dead_code,
    reason = "the Preview 3 filesystem remains deliberately unlinked"
)]
const PREOPENS_INTERFACE: &str = "wasi:filesystem/preopens@0.3.0";
#[allow(
    dead_code,
    reason = "the Preview 3 filesystem remains deliberately unlinked"
)]
const DESCRIPTOR: &str = "descriptor";

#[allow(
    dead_code,
    reason = "the complete interface remains unlinked until typed directory streams are designed"
)]
mod descriptors;
mod records;
#[allow(
    dead_code,
    reason = "the complete interface remains unlinked until typed directory streams are designed"
)]
mod resources;
#[allow(
    dead_code,
    reason = "the complete interface remains unlinked until typed directory streams are designed"
)]
mod streams;
#[cfg(test)]
mod test_support;
mod values;

#[allow(
    dead_code,
    reason = "registration waits for a typed directory-stream middleware design"
)]
fn add(linker: &mut wasmtime::component::Linker<crate::engine::StoreData>) -> wasmtime::Result<()> {
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
    fn available_filesystem_gates_register_without_linking() {
        let mut config = wasmtime::Config::new();
        config
            .wasm_component_model_async(true)
            .concurrency_support(true);
        let engine = wasmtime::Engine::new(&config).unwrap();
        let mut linker = wasmtime::component::Linker::new(&engine);
        add(&mut linker).unwrap();
    }
}
