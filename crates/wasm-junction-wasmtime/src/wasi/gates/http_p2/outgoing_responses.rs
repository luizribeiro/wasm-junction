#[allow(
    clippy::wildcard_imports,
    reason = "outgoing response gates share the parent HTTP gate machinery"
)]
use super::*;

fn validate_outparam(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    let [Val::Resource(param), response, ..] = values else {
        return Err(shape("response-outparam arguments"));
    };
    codec::validate_owned::<HostResponseOutparam>(param, store)?;
    if let Val::Result(Ok(Some(response))) = response {
        let Val::Resource(response) = response.as_ref() else {
            return Err(shape("outgoing-response"));
        };
        codec::validate_owned::<HostOutgoingResponse>(response, store)?;
    }
    Ok(())
}

fn response_body(
    store: &mut StoreData,
    response: Resource<HostOutgoingResponse>,
) -> wasmtime::Result<Result<Resource<HostOutgoingBody>, ()>> {
    let parent = response.rep();
    let body = OutgoingResponseApi::body(&mut views::http(store), response)?;
    if let Ok(body) = &body {
        copy_handle_context(store, parent, body.rep());
    }
    Ok(body)
}

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    gate_http!(linker, "[constructor]outgoing-response", OutgoingResponseApi::new,
        validate_owned_fields,
        (headers: Resource<FieldMap>) -> Resource<HostOutgoingResponse>);
    gate_http!(linker, "[method]outgoing-response.status-code", OutgoingResponseApi::status_code,
        validate_outgoing_response, (response: Resource<HostOutgoingResponse>) -> u16);
    gate_http_result!(linker, "[method]outgoing-response.set-status-code",
        OutgoingResponseApi::set_status_code, validate_outgoing_response, codec::convert_plain, (),
        (response: Resource<HostOutgoingResponse>, status: u16) -> Result<(), ()>);
    gate_http!(linker, "[method]outgoing-response.headers", OutgoingResponseApi::headers,
        validate_outgoing_response,
        (response: Resource<HostOutgoingResponse>) -> Resource<FieldMap>);
    gate_http_result!(linker, "[method]outgoing-response.body", store, response_body,
        validate_outgoing_response, codec::convert_plain, (),
        (response: Resource<HostOutgoingResponse>) -> Result<Resource<HostOutgoingBody>, ()>);
    gate_http_unit!(linker, "[static]response-outparam.set", ResponseOutparamApi::set,
        validate_outparam,
        (param: Resource<HostResponseOutparam>,
         response: Result<Resource<HostOutgoingResponse>, ErrorCode>));
    Ok(())
}
