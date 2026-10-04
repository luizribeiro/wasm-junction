#[allow(
    clippy::wildcard_imports,
    reason = "body gates share the parent HTTP gate machinery"
)]
use super::*;

fn request_body(
    store: &mut StoreData,
    request: Resource<HostOutgoingRequest>,
) -> wasmtime::Result<Result<Resource<HostOutgoingBody>, ()>> {
    let context = context::read(store, &request)?;
    let body = OutgoingRequestApi::body(&mut views::http(store), request)?;
    if let Ok(body) = &body {
        store.set_wasi_handle_context(body.rep(), context::value(context));
    }
    Ok(body)
}

fn outgoing_body_write(
    store: &mut StoreData,
    body: Resource<HostOutgoingBody>,
) -> wasmtime::Result<Result<Resource<DynOutputStream>, ()>> {
    let parent = body.rep();
    let stream = OutgoingBodyApi::write(&mut views::http(store), body)?;
    if let Ok(stream) = &stream {
        copy_handle_context(store, parent, stream.rep());
        open_channel(stream, store, ChannelDirection::GuestToHost)?;
    }
    Ok(stream)
}

fn incoming_body_stream(
    store: &mut StoreData,
    body: Resource<HostIncomingBody>,
) -> wasmtime::Result<Result<Resource<DynInputStream>, ()>> {
    let parent = body.rep();
    let stream = IncomingBodyApi::stream(&mut views::http(store), body)?;
    if let Ok(stream) = &stream {
        copy_handle_context(store, parent, stream.rep());
        open_channel(stream, store, ChannelDirection::HostToGuest)?;
    }
    Ok(stream)
}

fn incoming_body_finish(
    store: &mut StoreData,
    body: Resource<HostIncomingBody>,
) -> wasmtime::Result<Resource<HostFutureTrailers>> {
    let parent = body.rep();
    let trailers = IncomingBodyApi::finish(&mut views::http(store), body)?;
    copy_handle_context(store, parent, trailers.rep());
    Ok(trailers)
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_http_result!(linker, "[method]outgoing-request.body", store, request_body,
        validate_request, codec::convert_plain, (),
        (request: Resource<HostOutgoingRequest>) -> Result<Resource<HostOutgoingBody>, ()>);
    gate_http_result!(linker, "[method]outgoing-body.write", store, outgoing_body_write,
        validate_outgoing_body, codec::convert_plain, (),
        (body: Resource<HostOutgoingBody>) -> Result<Resource<DynOutputStream>, ()>);
    gate_http_result!(linker, "[method]incoming-body.stream", store, incoming_body_stream,
        validate_incoming_body, codec::convert_plain, (),
        (body: Resource<HostIncomingBody>) -> Result<Resource<DynInputStream>, ()>);
    gate_http!(linker, "[static]incoming-body.finish", store, incoming_body_finish,
        validate_owned_incoming_body,
        (body: Resource<HostIncomingBody>) -> Resource<HostFutureTrailers>);
    gate_http_result!(linker, "[static]outgoing-body.finish", OutgoingBodyApi::finish,
        validate_outgoing_body_finish, codec::convert_http, ErrorCode::HttpRequestDenied,
        (body: Resource<HostOutgoingBody>, trailers: Option<Resource<FieldMap>>)
            -> Result<(), ErrorCode>);
    Ok(())
}
