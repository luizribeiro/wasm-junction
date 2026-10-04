#[allow(
    clippy::wildcard_imports,
    reason = "HTTP gates share the parent module's private gate machinery"
)]
use super::*;
use codec::{FromHttpVal, HttpResource, ToHttpVal, gate_http, gate_http_drop, gate_http_result};
use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::{p2::DynInputStream, p2::DynOutputStream, p2::DynPollable};
use wasmtime_wasi_http::p2::bindings::http::types::{
    ErrorCode, HeaderError, HostFields, HostFutureIncomingResponse as FutureIncomingResponseApi,
    HostFutureTrailers as FutureTrailersApi, HostIncomingBody as IncomingBodyApi,
    HostIncomingRequest as IncomingRequestApi, HostIncomingResponse as IncomingResponseApi,
    HostOutgoingBody as OutgoingBodyApi, HostOutgoingRequest as OutgoingRequestApi,
    HostOutgoingResponse as OutgoingResponseApi, HostRequestOptions as RequestOptionsApi,
    HostResponseOutparam as ResponseOutparamApi, Method, Scheme,
};
use wasmtime_wasi_http::p2::body::{HostFutureTrailers, HostIncomingBody, HostOutgoingBody};
use wasmtime_wasi_http::p2::types::{
    HostFutureIncomingResponse, HostIncomingRequest, HostIncomingResponse, HostOutgoingRequest,
    HostOutgoingResponse, HostResponseOutparam,
};
use wasmtime_wasi_http::{FieldMap, RequestOptions};

use crate::engine::StoreData;

mod bodies;
mod codec;
mod context;
mod error_values;
mod outgoing;
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

macro_rules! validator {
    ($name:ident, $ty:ty) => {
        fn $name(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
            codec::validate_borrowed::<$ty>(
                values
                    .first()
                    .ok_or_else(|| shape(<$ty as HttpResource>::NAME))?,
                store,
            )
        }
    };
}

validator!(validate_request, HostOutgoingRequest);
validator!(validate_options, RequestOptions);
validator!(validate_outgoing_body, HostOutgoingBody);
validator!(validate_incoming_body, HostIncomingBody);

fn validate_owned_fields(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    let Some(Val::Resource(resource)) = values.first() else {
        return Err(shape("fields"));
    };
    codec::validate_owned::<FieldMap>(resource, store)
}

fn validate_owned_incoming_body(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    let Some(Val::Resource(resource)) = values.first() else {
        return Err(shape("incoming-body"));
    };
    codec::validate_owned::<HostIncomingBody>(resource, store)
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(OUTGOING_HANDLER)?;
    add_drops(linker)?;
    add_fields(linker)?;
    add_outgoing_request(linker)?;
    add_request_options(linker)?;
    bodies::add(linker)?;
    outgoing::add(linker)
}

fn add_drops(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_http_drop!(linker, "fields", FieldMap, HostFields::drop);
    gate_http_drop!(
        linker,
        "incoming-request",
        HostIncomingRequest,
        IncomingRequestApi::drop
    );
    gate_http_drop!(
        linker,
        "outgoing-request",
        HostOutgoingRequest,
        OutgoingRequestApi::drop
    );
    gate_http_drop!(
        linker,
        "request-options",
        RequestOptions,
        RequestOptionsApi::drop
    );
    gate_http_drop!(
        linker,
        "response-outparam",
        HostResponseOutparam,
        ResponseOutparamApi::drop
    );
    gate_http_drop!(
        linker,
        "incoming-response",
        HostIncomingResponse,
        IncomingResponseApi::drop
    );
    gate_http_drop!(
        linker,
        "incoming-body",
        HostIncomingBody,
        IncomingBodyApi::drop
    );
    gate_http_drop!(
        linker,
        "future-trailers",
        HostFutureTrailers,
        FutureTrailersApi::drop
    );
    gate_http_drop!(
        linker,
        "outgoing-response",
        HostOutgoingResponse,
        OutgoingResponseApi::drop
    );
    gate_http_drop!(
        linker,
        "outgoing-body",
        HostOutgoingBody,
        OutgoingBodyApi::drop
    );
    gate_http_drop!(
        linker,
        "future-incoming-response",
        HostFutureIncomingResponse,
        FutureIncomingResponseApi::drop
    );
    Ok(())
}

fn add_fields(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_http!(linker, "[constructor]fields", HostFields::new, no_resource_validation,
        () -> Resource<FieldMap>);
    gate_http_result!(linker, "[static]fields.from-list", HostFields::from_list,
        no_resource_validation, codec::convert_header, HeaderError::Forbidden,
        (entries: Vec<(String, Vec<u8>)>) -> Result<Resource<FieldMap>, HeaderError>);
    gate_http!(linker, "[method]fields.get", HostFields::get, validate_fields,
        (fields: Resource<FieldMap>, name: String) -> Vec<Vec<u8>>);
    gate_http!(linker, "[method]fields.has", HostFields::has, validate_fields,
        (fields: Resource<FieldMap>, name: String) -> bool);
    gate_http_result!(linker, "[method]fields.set", HostFields::set, validate_fields,
        codec::convert_header, HeaderError::Forbidden,
        (fields: Resource<FieldMap>, name: String, values: Vec<Vec<u8>>) -> Result<(), HeaderError>);
    gate_http_result!(linker, "[method]fields.delete", HostFields::delete, validate_fields,
        codec::convert_header, HeaderError::Forbidden,
        (fields: Resource<FieldMap>, name: String) -> Result<(), HeaderError>);
    gate_http_result!(linker, "[method]fields.append", HostFields::append, validate_fields,
        codec::convert_header, HeaderError::Forbidden,
        (fields: Resource<FieldMap>, name: String, value: Vec<u8>) -> Result<(), HeaderError>);
    gate_http!(linker, "[method]fields.entries", HostFields::entries, validate_fields,
        (fields: Resource<FieldMap>) -> Vec<(String, Vec<u8>)>);
    gate_http!(linker, "[method]fields.clone", HostFields::clone, validate_fields,
        (fields: Resource<FieldMap>) -> Resource<FieldMap>);
    Ok(())
}

fn add_outgoing_request(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_http!(linker, "[constructor]outgoing-request", OutgoingRequestApi::new,
        validate_owned_fields,
        (headers: Resource<FieldMap>) -> Resource<HostOutgoingRequest>);
    gate_http!(linker, "[method]outgoing-request.method", OutgoingRequestApi::method,
        validate_request, (request: Resource<HostOutgoingRequest>) -> Method);
    gate_http_result!(linker, "[method]outgoing-request.set-method", OutgoingRequestApi::set_method,
        validate_request, codec::convert_plain, (),
        (request: Resource<HostOutgoingRequest>, method: Method) -> Result<(), ()>);
    gate_http!(linker, "[method]outgoing-request.path-with-query", OutgoingRequestApi::path_with_query,
        validate_request, (request: Resource<HostOutgoingRequest>) -> Option<String>);
    gate_http_result!(linker, "[method]outgoing-request.set-path-with-query",
        OutgoingRequestApi::set_path_with_query, validate_request, codec::convert_plain, (),
        (request: Resource<HostOutgoingRequest>, path: Option<String>) -> Result<(), ()>);
    gate_http!(linker, "[method]outgoing-request.scheme", OutgoingRequestApi::scheme,
        validate_request, (request: Resource<HostOutgoingRequest>) -> Option<Scheme>);
    gate_http_result!(linker, "[method]outgoing-request.set-scheme", OutgoingRequestApi::set_scheme,
        validate_request, codec::convert_plain, (),
        (request: Resource<HostOutgoingRequest>, scheme: Option<Scheme>) -> Result<(), ()>);
    gate_http!(linker, "[method]outgoing-request.authority", OutgoingRequestApi::authority,
        validate_request, (request: Resource<HostOutgoingRequest>) -> Option<String>);
    gate_http_result!(linker, "[method]outgoing-request.set-authority",
        OutgoingRequestApi::set_authority, validate_request, codec::convert_plain, (),
        (request: Resource<HostOutgoingRequest>, authority: Option<String>) -> Result<(), ()>);
    gate_http!(linker, "[method]outgoing-request.headers", OutgoingRequestApi::headers,
        validate_request, (request: Resource<HostOutgoingRequest>) -> Resource<FieldMap>);
    Ok(())
}

fn add_request_options(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_http!(linker, "[constructor]request-options", RequestOptionsApi::new,
        no_resource_validation, () -> Resource<RequestOptions>);
    gate_http!(linker, "[method]request-options.connect-timeout", RequestOptionsApi::connect_timeout,
        validate_options, (options: Resource<RequestOptions>) -> Option<u64>);
    gate_http_result!(linker, "[method]request-options.set-connect-timeout",
        RequestOptionsApi::set_connect_timeout, validate_options, codec::convert_plain, (),
        (options: Resource<RequestOptions>, duration: Option<u64>) -> Result<(), ()>);
    gate_http!(linker, "[method]request-options.first-byte-timeout",
        RequestOptionsApi::first_byte_timeout, validate_options,
        (options: Resource<RequestOptions>) -> Option<u64>);
    gate_http_result!(linker, "[method]request-options.set-first-byte-timeout",
        RequestOptionsApi::set_first_byte_timeout, validate_options, codec::convert_plain, (),
        (options: Resource<RequestOptions>, duration: Option<u64>) -> Result<(), ()>);
    gate_http!(linker, "[method]request-options.between-bytes-timeout",
        RequestOptionsApi::between_bytes_timeout, validate_options,
        (options: Resource<RequestOptions>) -> Option<u64>);
    gate_http_result!(linker, "[method]request-options.set-between-bytes-timeout",
        RequestOptionsApi::set_between_bytes_timeout, validate_options, codec::convert_plain, (),
        (options: Resource<RequestOptions>, duration: Option<u64>) -> Result<(), ()>);
    Ok(())
}
