use wasmtime::component::Linker;

use crate::engine::StoreData;

mod values;

const TYPES: &str = "wasi:http/types@0.2.12";
const OUTGOING_HANDLER: &str = "wasi:http/outgoing-handler@0.2.12";

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(TYPES)?;
    linker.instance(OUTGOING_HANDLER)?;
    Ok(())
}
