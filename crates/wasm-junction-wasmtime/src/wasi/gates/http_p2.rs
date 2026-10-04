#[allow(
    clippy::wildcard_imports,
    reason = "HTTP gates share the parent module's private gate machinery"
)]
use super::*;
use codec::{FromHttpVal, HttpResource, ToHttpVal, gate_http};
use wasmtime::component::{Linker, Resource};
use wasmtime_wasi_http::FieldMap;
use wasmtime_wasi_http::p2::bindings::http::types::HostFields;

use crate::engine::StoreData;

mod codec;
mod values;

const TYPES: &str = "wasi:http/types@0.2.12";
const OUTGOING_HANDLER: &str = "wasi:http/outgoing-handler@0.2.12";

impl HttpResource for FieldMap {
    const INTERFACE: &'static str = TYPES;
    const NAME: &'static str = "fields";
}

fn validate_fields(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    let Some(Val::Resource(resource)) = values.first() else {
        return Err(CallError::refused("expected fields handle"));
    };
    let invocation = store
        .context
        .invocation_id()
        .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
    validate_resource_for_invocation(
        resource,
        TYPES,
        "fields",
        ResourceOwnership::Borrow,
        invocation,
    )?;
    store
        .wasi_table()
        .get(&Resource::<FieldMap>::new_borrow(resource.id()))
        .map_err(|_| CallError::refused(format!("unknown fields handle {}", resource.id())))?;
    Ok(())
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(OUTGOING_HANDLER)?;
    gate_http!(linker, "[constructor]fields", HostFields::new, no_resource_validation,
        () -> Resource<FieldMap>);
    gate_http!(linker, "[method]fields.get", HostFields::get, validate_fields,
        (fields: Resource<FieldMap>, name: String) -> Vec<Vec<u8>>);
    gate_http!(linker, "[method]fields.has", HostFields::has, validate_fields,
        (fields: Resource<FieldMap>, name: String) -> bool);
    gate_http!(linker, "[method]fields.entries", HostFields::entries, validate_fields,
        (fields: Resource<FieldMap>) -> Vec<(String, Vec<u8>)>);
    gate_http!(linker, "[method]fields.clone", HostFields::clone, validate_fields,
        (fields: Resource<FieldMap>) -> Resource<FieldMap>);
    Ok(())
}
