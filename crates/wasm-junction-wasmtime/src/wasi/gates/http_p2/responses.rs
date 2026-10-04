#[allow(
    clippy::wildcard_imports,
    reason = "response gates share the parent HTTP gate machinery"
)]
use super::*;

type FutureResponseResult = Option<Result<Result<Resource<HostIncomingResponse>, ErrorCode>, ()>>;
type FutureTrailersResult = Option<Result<Result<Option<Resource<FieldMap>>, ErrorCode>, ()>>;

fn future_get(
    store: &mut StoreData,
    future: Resource<HostFutureIncomingResponse>,
) -> wasmtime::Result<FutureResponseResult> {
    let parent = future.rep();
    let result = FutureIncomingResponseApi::get(&mut views::http(store), future)?;
    if let Some(Ok(Ok(response))) = &result {
        copy_handle_context(store, parent, response.rep());
    }
    Ok(result)
}

fn response_consume(
    store: &mut StoreData,
    response: Resource<HostIncomingResponse>,
) -> wasmtime::Result<Result<Resource<HostIncomingBody>, ()>> {
    let parent = response.rep();
    let body = IncomingResponseApi::consume(&mut views::http(store), response)?;
    if let Ok(body) = &body {
        copy_handle_context(store, parent, body.rep());
    }
    Ok(body)
}

fn subscribe_future(
    store: &mut StoreData,
    future: Resource<HostFutureIncomingResponse>,
) -> wasmtime::Result<Resource<DynPollable>> {
    let parent = future.rep();
    let pollable = FutureIncomingResponseApi::subscribe(&mut views::http(store), future)?;
    copy_handle_context(store, parent, pollable.rep());
    Ok(pollable)
}

fn subscribe_trailers(
    store: &mut StoreData,
    future: Resource<HostFutureTrailers>,
) -> wasmtime::Result<Resource<DynPollable>> {
    let parent = future.rep();
    let pollable = FutureTrailersApi::subscribe(&mut views::http(store), future)?;
    copy_handle_context(store, parent, pollable.rep());
    Ok(pollable)
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_http!(linker, "[method]future-incoming-response.get", store, future_get,
        validate_future_response,
        (future: Resource<HostFutureIncomingResponse>) -> FutureResponseResult);
    gate_http!(linker, "[method]future-incoming-response.subscribe", store, subscribe_future,
        validate_future_response,
        (future: Resource<HostFutureIncomingResponse>) -> Resource<DynPollable>);
    gate_http!(linker, "[method]incoming-response.status", IncomingResponseApi::status,
        validate_incoming_response, (response: Resource<HostIncomingResponse>) -> u16);
    gate_http!(linker, "[method]incoming-response.headers", IncomingResponseApi::headers,
        validate_incoming_response,
        (response: Resource<HostIncomingResponse>) -> Resource<FieldMap>);
    gate_http_result!(linker, "[method]incoming-response.consume", store, response_consume,
        validate_incoming_response, codec::convert_plain, (),
        (response: Resource<HostIncomingResponse>) -> Result<Resource<HostIncomingBody>, ()>);
    gate_http!(linker, "[method]future-trailers.subscribe", store, subscribe_trailers,
        validate_future_trailers,
        (future: Resource<HostFutureTrailers>) -> Resource<DynPollable>);
    gate_http!(linker, "[method]future-trailers.get", FutureTrailersApi::get,
        validate_future_trailers,
        (future: Resource<HostFutureTrailers>) -> FutureTrailersResult);
    Ok(())
}
