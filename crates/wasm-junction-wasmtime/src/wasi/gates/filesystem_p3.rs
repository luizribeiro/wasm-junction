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
