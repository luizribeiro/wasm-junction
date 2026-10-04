#[allow(
    clippy::wildcard_imports,
    reason = "HTTP gates share the parent module's private gate machinery"
)]
use super::*;
use codec::{FromHttpVal, HttpResource, ToHttpVal, gate_http};
use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::{p2::DynInputStream, p2::DynOutputStream, p2::DynPollable};
use wasmtime_wasi_http::p2::bindings::http::types::HostFields;
use wasmtime_wasi_http::p2::body::{HostFutureTrailers, HostIncomingBody, HostOutgoingBody};
use wasmtime_wasi_http::p2::types::{
    HostFutureIncomingResponse, HostIncomingRequest, HostIncomingResponse, HostOutgoingRequest,
    HostOutgoingResponse, HostResponseOutparam,
};
use wasmtime_wasi_http::{FieldMap, RequestOptions};

use crate::engine::StoreData;

mod codec;
mod values;

const TYPES: &str = "wasi:http/types@0.2.12";
const OUTGOING_HANDLER: &str = "wasi:http/outgoing-handler@0.2.12";

macro_rules! http_resource {
    ($ty:ty, $name:literal) => {
        http_resource!($ty, TYPES, $name);
    };
    ($ty:ty, $interface:expr, $name:literal) => {
        impl HttpResource for $ty {
            const INTERFACE: &'static str = $interface;
            const NAME: &'static str = $name;
        }
    };
}

http_resource!(FieldMap, "fields");
http_resource!(HostIncomingRequest, "incoming-request");
http_resource!(HostOutgoingRequest, "outgoing-request");
http_resource!(RequestOptions, "request-options");
http_resource!(HostResponseOutparam, "response-outparam");
http_resource!(HostIncomingResponse, "incoming-response");
http_resource!(HostIncomingBody, "incoming-body");
http_resource!(HostFutureTrailers, "future-trailers");
http_resource!(HostOutgoingResponse, "outgoing-response");
http_resource!(HostOutgoingBody, "outgoing-body");
http_resource!(HostFutureIncomingResponse, "future-incoming-response");
http_resource!(DynInputStream, STREAMS_INTERFACE, "input-stream");
http_resource!(DynOutputStream, STREAMS_INTERFACE, "output-stream");
http_resource!(DynPollable, POLLABLE_INTERFACE, "pollable");

fn validate_fields(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    codec::validate_borrowed::<FieldMap>(values.first().ok_or_else(|| shape("fields"))?, store)
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
