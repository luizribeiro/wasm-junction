use super::context::{
    Headers, apply as apply_context, read as request_context, value as context_value,
};
#[allow(
    clippy::wildcard_imports,
    reason = "the outgoing gate shares the parent HTTP gate machinery"
)]
use super::*;
use wasm_junction_core::WasiSettings;
use wasmtime_wasi_http::p2::bindings::http::outgoing_handler;

pub(super) fn add(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.instance(OUTGOING_HANDLER)?.func_wrap_async(
        "handle",
        |mut store,
         (request, options): (
            Resource<HostOutgoingRequest>,
            Option<Resource<RequestOptions>>,
        )| {
            Box::new(async move {
                let invocation = store
                    .data()
                    .context
                    .invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let (method, scheme, authority, path, headers) =
                    request_context(store.data_mut(), &request)?;
                let args = scope_values(
                    vec![
                        request.to_http_val(),
                        method.to_http_val(),
                        scheme.to_http_val(),
                        authority.to_http_val(),
                        path.to_http_val(),
                        headers.to_http_val(),
                        options.to_http_val(),
                    ],
                    invocation,
                );
                let real: Real = |mut store, args| {
                    Box::pin(async move {
                        let [request, method, scheme, authority, path, headers, options] =
                            <[Val; 7]>::try_from(args)
                                .map_err(|_| shape("outgoing HTTP context"))?;
                        let Val::Resource(resource) = &request else {
                            return Err(shape("outgoing-request"));
                        };
                        codec::validate_owned::<HostOutgoingRequest>(resource, store.data_mut())?;
                        let request = Resource::from_http_val(request)?;
                        let options = Option::<Resource<RequestOptions>>::from_http_val(options)?;
                        let current = store
                            .data()
                            .context
                            .invocation_id()
                            .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                        if let Some(options) = &options {
                            codec::validate_owned::<RequestOptions>(
                                &JunctionResource::__owned_for_invocation(
                                    TYPES,
                                    "request-options",
                                    options.rep(),
                                    current,
                                ),
                                store.data_mut(),
                            )?;
                        }
                        let context = (
                            Method::from_http_val(method)?,
                            Option::from_http_val(scheme)?,
                            Option::from_http_val(authority)?,
                            Option::from_http_val(path)?,
                            Headers::from_http_val(headers)?,
                        );
                        let enabled = store
                            .data()
                            .context
                            .settings()
                            .get::<WasiSettings>()
                            .is_some_and(WasiSettings::network_enabled);
                        if !enabled {
                            return Err(CallError::refused("outgoing HTTP is disabled"));
                        }
                        apply_context(store.data_mut(), &request, context.clone())
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        let result = codec::convert_http(outgoing_handler::Host::handle(
                            &mut views::http(store.data_mut()),
                            request,
                            options,
                        ))?;
                        if let Ok(future) = &result {
                            store
                                .data_mut()
                                .set_wasi_handle_context(future.rep(), context_value(context));
                        }
                        Ok(vec![result.to_http_val()])
                    })
                };
                let outcome =
                    trampoline::gate(&mut store, OUTGOING_HANDLER, "handle", args, real).await;
                Ok((codec::finish_result::<
                    Resource<HostFutureIncomingResponse>,
                    ErrorCode,
                >(outcome, ErrorCode::HttpRequestDenied)?,))
            })
        },
    )?;
    Ok(())
}
