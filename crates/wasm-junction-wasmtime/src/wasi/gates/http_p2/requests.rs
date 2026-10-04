#[allow(
    clippy::wildcard_imports,
    reason = "request gates share the parent HTTP gate machinery"
)]
use super::*;

fn request_headers(
    store: &mut StoreData,
    request: Resource<HostIncomingRequest>,
) -> wasmtime::Result<Resource<FieldMap>> {
    let parent = request.rep();
    let headers = IncomingRequestApi::headers(&mut views::http(store), request)?;
    copy_handle_context(store, parent, headers.rep());
    Ok(headers)
}

fn request_consume(
    store: &mut StoreData,
    request: Resource<HostIncomingRequest>,
) -> wasmtime::Result<Result<Resource<HostIncomingBody>, ()>> {
    let parent = request.rep();
    let body = IncomingRequestApi::consume(&mut views::http(store), request)?;
    if let Ok(body) = &body {
        copy_handle_context(store, parent, body.rep());
    }
    Ok(body)
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_http!(linker, "[method]incoming-request.method", IncomingRequestApi::method,
        validate_incoming_request, (request: Resource<HostIncomingRequest>) -> Method);
    gate_http!(linker, "[method]incoming-request.path-with-query",
        IncomingRequestApi::path_with_query, validate_incoming_request,
        (request: Resource<HostIncomingRequest>) -> Option<String>);
    gate_http!(linker, "[method]incoming-request.scheme", IncomingRequestApi::scheme,
        validate_incoming_request,
        (request: Resource<HostIncomingRequest>) -> Option<Scheme>);
    gate_http!(linker, "[method]incoming-request.authority", IncomingRequestApi::authority,
        validate_incoming_request,
        (request: Resource<HostIncomingRequest>) -> Option<String>);
    gate_http!(linker, "[method]incoming-request.headers", store, request_headers,
        validate_incoming_request,
        (request: Resource<HostIncomingRequest>) -> Resource<FieldMap>);
    gate_http_result!(linker, "[method]incoming-request.consume", store, request_consume,
        validate_incoming_request, codec::convert_plain, (),
        (request: Resource<HostIncomingRequest>) -> Result<Resource<HostIncomingBody>, ()>);
    Ok(())
}
