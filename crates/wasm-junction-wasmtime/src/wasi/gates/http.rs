#[allow(
    clippy::wildcard_imports,
    reason = "HTTP gates share the parent module's private gate machinery"
)]
use super::*;
use wasmtime::component::{Access, ComponentType, FutureReader, StreamReader};
use wasmtime::{AsContextMut, StoreContextMut};
use wasmtime_wasi_http::p3::bindings::http::types::{
    ErrorCode, Fields, HeaderError, Method, Request, RequestOptions, RequestOptionsError, Response,
    Scheme,
};

#[cfg(test)]
use wasmtime_wasi_http::p3::bindings::http::types::{DnsErrorPayload, FieldSizePayload};

const TYPES: &str = "wasi:http/types@0.3.0";
const FIELDS_NAME: &str = "fields";
const REQUEST_OPTIONS_NAME: &str = "request-options";
const REQUEST_NAME: &str = "request";
const RESPONSE_NAME: &str = "response";

macro_rules! resource {
    ($ty:ty, $name:literal) => {
        impl WitResource for $ty {
            const INTERFACE: &'static str = TYPES;
            const NAME: &'static str = $name;
        }
    };
}

resource!(Fields, "fields");
resource!(Request, "request");
resource!(RequestOptions, "request-options");
resource!(Response, "response");

fn validate_fields(values: &[Val], store: &mut crate::engine::StoreData) -> Result<(), CallError> {
    validate_borrowed::<Fields>(values.first().ok_or_else(|| shape(FIELDS_NAME))?, store)
}

fn drop_fields(store: &mut StoreData, fields: Resource<Fields>) -> wasmtime::Result<()> {
    let mut view = views::http(store);
    wasmtime_wasi_http::p3::bindings::http::types::HostFields::drop(&mut view, fields)
}

fn validate_request_options(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_borrowed::<RequestOptions>(
        values.first().ok_or_else(|| shape(REQUEST_OPTIONS_NAME))?,
        store,
    )
}

fn validate_request(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_borrowed::<Request>(values.first().ok_or_else(|| shape("request"))?, store)
}

fn validate_response(values: &[Val], store: &mut StoreData) -> Result<(), CallError> {
    validate_borrowed::<Response>(values.first().ok_or_else(|| shape("response"))?, store)
}

fn lift_future_plain<T: ComponentType + 'static>(
    store: &mut StoreContextMut<'_, StoreData>,
    future: FutureReader<T>,
) -> wasmtime::Result<Val> {
    let future = future.try_into_future_any(store.as_context_mut())?;
    crate::engine::lift_future(future, store.data_mut()).map(Val::Future)
}

fn lower_future_plain<T: ComponentType + 'static>(
    store: &mut StoreContextMut<'_, StoreData>,
    value: Val,
) -> wasmtime::Result<FutureReader<T>> {
    let Val::Future(future) = value else {
        return Err(wasmtime::Error::new(shape("future")));
    };
    let future = crate::engine::lower_future(&future, store.data_mut())?;
    FutureReader::try_from_future_any(future)
}

fn lift_optional_stream_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    stream: Option<StreamReader<u8>>,
) -> wasmtime::Result<Val> {
    stream
        .map(|stream| lift_stream_plain(store, stream))
        .transpose()
        .map(|stream| Val::Option(stream.map(Box::new)))
}

fn lift_stream_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    stream: StreamReader<u8>,
) -> wasmtime::Result<Val> {
    let stream = stream.try_into_stream_any(store.as_context_mut())?;
    crate::streams::lift_stream(stream, store.as_context_mut()).map(Val::Stream)
}

fn lower_optional_stream_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    value: Val,
) -> wasmtime::Result<Option<StreamReader<u8>>> {
    let Val::Option(stream) = value else {
        return Err(wasmtime::Error::new(shape("optional stream")));
    };
    stream
        .map(|stream| lower_stream_plain(store, *stream))
        .transpose()
}

fn lower_stream_plain(
    store: &mut StoreContextMut<'_, StoreData>,
    value: Val,
) -> wasmtime::Result<StreamReader<u8>> {
    let Val::Stream(stream) = value else {
        return Err(wasmtime::Error::new(shape("stream")));
    };
    let stream = crate::streams::lower_stream(stream, store.as_context_mut())?;
    StreamReader::try_from_stream_any(stream)
}

fn validate_owned_arg<T: WitResource>(
    value: &Val,
    accessor: &wasmtime::component::Accessor<StoreData>,
) -> Result<(), CallError> {
    let Val::Resource(resource) = value else {
        return Err(shape(T::NAME));
    };
    accessor.with(|mut access| validate_owned::<T>(resource, access.get()))
}

type TransferResult = Result<(), ErrorCode>;
type TrailersResult = Result<Option<Resource<Fields>>, ErrorCode>;
type Headers = Vec<(String, Vec<u8>)>;
type RequestContext = (
    Method,
    Option<Scheme>,
    Option<String>,
    Option<String>,
    Headers,
);
type RequestNewParams = (
    Resource<Fields>,
    Option<StreamReader<u8>>,
    FutureReader<TrailersResult>,
    Option<Resource<RequestOptions>>,
);

fn request_context(
    store: &mut StoreData,
    request: &Resource<Request>,
) -> wasmtime::Result<RequestContext> {
    use wasmtime_wasi_http::p3::bindings::http::types::{HostFields, HostRequest};

    let id = request.rep();
    let mut view = views::http(store);
    let method = HostRequest::get_method(&mut view, Resource::new_borrow(id))?;
    let scheme = HostRequest::get_scheme(&mut view, Resource::new_borrow(id))?;
    let authority = HostRequest::get_authority(&mut view, Resource::new_borrow(id))?;
    let path = HostRequest::get_path_with_query(&mut view, Resource::new_borrow(id))?;
    let fields = HostRequest::get_headers(&mut view, Resource::new_borrow(id))?;
    let headers = HostFields::copy_all(&mut view, Resource::new_borrow(fields.rep()))?;
    HostFields::drop(&mut view, fields)?;
    Ok((method, scheme, authority, path, headers))
}

fn apply_request_context(
    store: &mut StoreData,
    request: &Resource<Request>,
    method: Method,
    scheme: Option<Scheme>,
    authority: Option<String>,
    path: Option<String>,
    headers: Headers,
) -> wasmtime::Result<()> {
    use wasmtime_wasi_http::p3::bindings::http::types::{HostFields, HostRequest};

    let id = request.rep();
    let mut view = views::http(store);
    let valid = HostRequest::set_method(&mut view, Resource::new_borrow(id), method)?
        .and(HostRequest::set_scheme(
            &mut view,
            Resource::new_borrow(id),
            scheme,
        )?)
        .and(HostRequest::set_authority(
            &mut view,
            Resource::new_borrow(id),
            authority,
        )?)
        .and(HostRequest::set_path_with_query(
            &mut view,
            Resource::new_borrow(id),
            path,
        )?);
    if valid.is_err() {
        return Err(wasmtime::Error::msg(
            "middleware produced invalid HTTP request metadata",
        ));
    }
    let fields = HostFields::from_list(&mut view, headers)
        .map_err(|error| wasmtime::Error::msg(error.to_string()))?;
    let fields = view.table.delete(fields)?;
    view.table
        .get_mut(&Resource::<Request>::new_borrow(id))?
        .headers = fields;
    Ok(())
}

fn drop_request_options(
    store: &mut StoreData,
    options: Resource<RequestOptions>,
) -> wasmtime::Result<()> {
    let mut view = views::http(store);
    wasmtime_wasi_http::p3::bindings::http::types::HostRequestOptions::drop(&mut view, options)
}

#[expect(
    clippy::too_many_lines,
    reason = "keeping the WIT registrations together makes gate coverage auditable"
)]
pub(super) fn add(linker: &mut Linker<crate::engine::StoreData>) -> wasmtime::Result<()> {
    use wasmtime_wasi_http::p3::bindings::http::types::{
        HostFields, HostRequest, HostRequestOptions, HostRequestWithStore, HostResponse,
        HostResponseWithStore,
    };

    gate_drop!(
        linker,
        TYPES,
        FIELDS_NAME,
        "[drop]fields",
        Fields,
        store,
        None,
        drop_fields
    );
    gate!(linker, "wasi:http/types@0.3.0", "[constructor]fields", http, HostFields::new,
        plain, () -> Resource<Fields>);
    gate!(linker, "wasi:http/types@0.3.0", "[static]fields.from-list", http, HostFields::from_list,
        plain_result[HeaderError::Forbidden],
        (entries: Vec<(String, Vec<u8>)>) -> Result<Resource<Fields>, HeaderError>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]fields.get", http, HostFields::get,
        plain_with[validate_fields],
        (fields: Resource<Fields>, name: String) -> Vec<Vec<u8>>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]fields.has", http, HostFields::has,
        plain_with[validate_fields], (fields: Resource<Fields>, name: String) -> bool);
    gate!(linker, "wasi:http/types@0.3.0", "[method]fields.set", http, HostFields::set,
        plain_result_with[validate_fields, HeaderError::Forbidden],
        (fields: Resource<Fields>, name: String, values: Vec<Vec<u8>>) -> Result<(), HeaderError>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]fields.delete", http, HostFields::delete,
        plain_result_with[validate_fields, HeaderError::Forbidden],
        (fields: Resource<Fields>, name: String) -> Result<(), HeaderError>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]fields.get-and-delete", http, HostFields::get_and_delete,
        plain_result_with[validate_fields, HeaderError::Forbidden],
        (fields: Resource<Fields>, name: String) -> Result<Vec<Vec<u8>>, HeaderError>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]fields.append", http, HostFields::append,
        plain_result_with[validate_fields, HeaderError::Forbidden],
        (fields: Resource<Fields>, name: String, value: Vec<u8>) -> Result<(), HeaderError>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]fields.copy-all", http, HostFields::copy_all,
        plain_with[validate_fields],
        (fields: Resource<Fields>) -> Vec<(String, Vec<u8>)>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]fields.clone", http, HostFields::clone,
        plain_with[validate_fields], (fields: Resource<Fields>) -> Resource<Fields>);
    gate_drop!(
        linker,
        TYPES,
        REQUEST_OPTIONS_NAME,
        "[drop]request-options",
        RequestOptions,
        store,
        None,
        drop_request_options
    );
    gate!(linker, "wasi:http/types@0.3.0", "[constructor]request-options", http,
        HostRequestOptions::new, plain, () -> Resource<RequestOptions>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request-options.get-connect-timeout", http,
        HostRequestOptions::get_connect_timeout, plain_with[validate_request_options],
        (options: Resource<RequestOptions>) -> Option<u64>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request-options.set-connect-timeout", http,
        HostRequestOptions::set_connect_timeout,
        plain_result_with[validate_request_options, RequestOptionsError::NotSupported],
        (options: Resource<RequestOptions>, duration: Option<u64>) -> Result<(), RequestOptionsError>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request-options.get-first-byte-timeout", http,
        HostRequestOptions::get_first_byte_timeout, plain_with[validate_request_options],
        (options: Resource<RequestOptions>) -> Option<u64>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request-options.set-first-byte-timeout", http,
        HostRequestOptions::set_first_byte_timeout,
        plain_result_with[validate_request_options, RequestOptionsError::NotSupported],
        (options: Resource<RequestOptions>, duration: Option<u64>) -> Result<(), RequestOptionsError>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request-options.get-between-bytes-timeout", http,
        HostRequestOptions::get_between_bytes_timeout, plain_with[validate_request_options],
        (options: Resource<RequestOptions>) -> Option<u64>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request-options.set-between-bytes-timeout", http,
        HostRequestOptions::set_between_bytes_timeout,
        plain_result_with[validate_request_options, RequestOptionsError::NotSupported],
        (options: Resource<RequestOptions>, duration: Option<u64>) -> Result<(), RequestOptionsError>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request-options.clone", http,
        HostRequestOptions::clone, plain_with[validate_request_options],
        (options: Resource<RequestOptions>) -> Resource<RequestOptions>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.get-method", http,
        HostRequest::get_method, plain_with[validate_request],
        (request: Resource<Request>) -> Method);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.set-method", http,
        HostRequest::set_method, plain_with[validate_request],
        (request: Resource<Request>, method: Method) -> Result<(), ()>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.get-path-with-query", http,
        HostRequest::get_path_with_query, plain_with[validate_request],
        (request: Resource<Request>) -> Option<String>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.set-path-with-query", http,
        HostRequest::set_path_with_query, plain_with[validate_request],
        (request: Resource<Request>, path: Option<String>) -> Result<(), ()>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.get-scheme", http,
        HostRequest::get_scheme, plain_with[validate_request],
        (request: Resource<Request>) -> Option<Scheme>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.set-scheme", http,
        HostRequest::set_scheme, plain_with[validate_request],
        (request: Resource<Request>, scheme: Option<Scheme>) -> Result<(), ()>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.get-authority", http,
        HostRequest::get_authority, plain_with[validate_request],
        (request: Resource<Request>) -> Option<String>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.set-authority", http,
        HostRequest::set_authority, plain_with[validate_request],
        (request: Resource<Request>, authority: Option<String>) -> Result<(), ()>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.get-options", http,
        HostRequest::get_options, plain_with[validate_request],
        (request: Resource<Request>) -> Option<Resource<RequestOptions>>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]request.get-headers", http,
        HostRequest::get_headers, plain_with[validate_request],
        (request: Resource<Request>) -> Resource<Fields>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]response.get-status-code", http,
        HostResponse::get_status_code, plain_with[validate_response],
        (response: Resource<Response>) -> u16);
    gate!(linker, "wasi:http/types@0.3.0", "[method]response.set-status-code", http,
        HostResponse::set_status_code, plain_with[validate_response],
        (response: Resource<Response>, status: u16) -> Result<(), ()>);
    gate!(linker, "wasi:http/types@0.3.0", "[method]response.get-headers", http,
        HostResponse::get_headers, plain_with[validate_response],
        (response: Resource<Response>) -> Resource<Fields>);
    gate_concurrent_drop!(
        linker,
        TYPES,
        REQUEST_NAME,
        "[drop]request",
        Request,
        HostRequestWithStore::drop
    );
    gate_concurrent_drop!(
        linker,
        TYPES,
        RESPONSE_NAME,
        "[drop]response",
        Response,
        HostResponseWithStore::drop
    );
    add_request_new(linker)?;
    add_request_consume(linker)?;
    add_response_new(linker)?;
    add_response_consume(linker)?;
    add_send(linker)?;
    Ok(())
}

fn add_request_new(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    use wasmtime_wasi_http::p3::bindings::http::types::HostRequestWithStore;
    use wasmtime_wasi_http::{WasiHttp, WasiHttpView};

    linker.instance(TYPES)?.func_wrap_async(
        "[static]request.new",
        |mut store, (headers, contents, trailers, options): RequestNewParams| {
            Box::new(async move {
                let invocation = store
                    .data()
                    .context
                    .invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = vec![
                    scope(headers.to_val(), invocation),
                    lift_optional_stream_plain(&mut store, contents)?,
                    lift_future_plain(&mut store, trailers)?,
                    scope(options.to_val(), invocation),
                ];
                let real: Real = |mut store, args| {
                    Box::pin(async move {
                        let [headers, contents, trailers, options] = <[Val; 4]>::try_from(args)
                            .map_err(|_| shape("request.new arguments"))?;
                        let Val::Resource(header_resource) = &headers else {
                            return Err(shape(FIELDS_NAME));
                        };
                        validate_owned::<Fields>(header_resource, store.data_mut())?;
                        match &options {
                            Val::Option(None) => {}
                            Val::Option(Some(options)) => {
                                let Val::Resource(options) = options.as_ref() else {
                                    return Err(shape(REQUEST_OPTIONS_NAME));
                                };
                                validate_owned::<RequestOptions>(options, store.data_mut())?;
                            }
                            _ => return Err(shape("option")),
                        }
                        let headers = Resource::<Fields>::from_val(headers)?;
                        let contents = lower_optional_stream_plain(&mut store, contents)
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        let trailers = lower_future_plain(&mut store, trailers)
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        let options = Option::<Resource<RequestOptions>>::from_val(options)?;
                        let access = Access::<StoreData, WasiHttp>::new(
                            store.as_context_mut(),
                            WasiHttpView::http,
                        );
                        let (request, transferred) =
                            HostRequestWithStore::new(access, headers, contents, trailers, options)
                                .map_err(|error| CallError::trap(error.to_string()))?;
                        let invocation = store
                            .data()
                            .context
                            .invocation_id()
                            .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                        Ok(vec![Val::Tuple(vec![
                            scope(request.to_val(), invocation),
                            lift_future_plain(&mut store, transferred)
                                .map_err(|error| CallError::trap(error.to_string()))?,
                        ])])
                    })
                };
                let outcome =
                    trampoline::gate(&mut store, TYPES, "[static]request.new", args, real)
                        .await
                        .map_err(wasmtime::Error::new)?;
                let [Val::Tuple(values)] = outcome.as_slice() else {
                    return Err(wasmtime::Error::new(shape("resource and future")));
                };
                let [request, transferred] = values.as_slice() else {
                    return Err(wasmtime::Error::new(shape("resource and future")));
                };
                let request =
                    Resource::<Request>::from_val(request.clone()).map_err(wasmtime::Error::new)?;
                let transferred: FutureReader<TransferResult> =
                    lower_future_plain(&mut store, transferred.clone())?;
                Ok(((request, transferred),))
            })
        },
    )?;
    Ok(())
}

fn add_request_consume(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    use wasmtime_wasi_http::p3::bindings::http::types::HostRequestWithStore;
    use wasmtime_wasi_http::{WasiHttp, WasiHttpView};

    linker.instance(TYPES)?.func_wrap_async(
        "[static]request.consume-body",
        |mut store, (request, transferred): (Resource<Request>, FutureReader<TransferResult>)| {
            Box::new(async move {
                let invocation = store
                    .data()
                    .context
                    .invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = vec![
                    scope(request.to_val(), invocation),
                    lift_future_plain(&mut store, transferred)?,
                ];
                let real: Real = |mut store, args| {
                    Box::pin(async move {
                        let [request, transferred] = <[Val; 2]>::try_from(args)
                            .map_err(|_| shape("request.consume-body arguments"))?;
                        let Val::Resource(resource) = &request else {
                            return Err(shape(REQUEST_NAME));
                        };
                        validate_owned::<Request>(resource, store.data_mut())?;
                        let request = Resource::<Request>::from_val(request)?;
                        let transferred = lower_future_plain(&mut store, transferred)
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        let access = Access::<StoreData, WasiHttp>::new(
                            store.as_context_mut(),
                            WasiHttpView::http,
                        );
                        let (body, trailers) =
                            HostRequestWithStore::consume_body(access, request, transferred)
                                .map_err(|error| CallError::trap(error.to_string()))?;
                        Ok(vec![Val::Tuple(vec![
                            lift_stream_plain(&mut store, body)
                                .map_err(|error| CallError::trap(error.to_string()))?,
                            lift_future_plain(&mut store, trailers)
                                .map_err(|error| CallError::trap(error.to_string()))?,
                        ])])
                    })
                };
                let outcome = trampoline::gate(
                    &mut store,
                    TYPES,
                    "[static]request.consume-body",
                    args,
                    real,
                )
                .await
                .map_err(wasmtime::Error::new)?;
                let [Val::Tuple(values)] = outcome.as_slice() else {
                    return Err(wasmtime::Error::new(shape("body and trailers")));
                };
                let [body, trailers] = values.as_slice() else {
                    return Err(wasmtime::Error::new(shape("body and trailers")));
                };
                let body = lower_stream_plain(&mut store, body.clone())?;
                let trailers: FutureReader<TrailersResult> =
                    lower_future_plain(&mut store, trailers.clone())?;
                Ok(((body, trailers),))
            })
        },
    )?;
    Ok(())
}

fn add_response_new(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    use wasmtime_wasi_http::p3::bindings::http::types::HostResponseWithStore;
    use wasmtime_wasi_http::{WasiHttp, WasiHttpView};

    linker.instance(TYPES)?.func_wrap_async(
        "[static]response.new",
        |mut store,
         (headers, contents, trailers): (
            Resource<Fields>,
            Option<StreamReader<u8>>,
            FutureReader<TrailersResult>,
        )| {
            Box::new(async move {
                let invocation = store
                    .data()
                    .context
                    .invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = vec![
                    scope(headers.to_val(), invocation),
                    lift_optional_stream_plain(&mut store, contents)?,
                    lift_future_plain(&mut store, trailers)?,
                ];
                let real: Real = |mut store, args| {
                    Box::pin(async move {
                        let [headers, contents, trailers] = <[Val; 3]>::try_from(args)
                            .map_err(|_| shape("response.new arguments"))?;
                        let Val::Resource(resource) = &headers else {
                            return Err(shape(FIELDS_NAME));
                        };
                        validate_owned::<Fields>(resource, store.data_mut())?;
                        let headers = Resource::<Fields>::from_val(headers)?;
                        let contents = lower_optional_stream_plain(&mut store, contents)
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        let trailers = lower_future_plain(&mut store, trailers)
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        let access = Access::<StoreData, WasiHttp>::new(
                            store.as_context_mut(),
                            WasiHttpView::http,
                        );
                        let (response, transferred) =
                            HostResponseWithStore::new(access, headers, contents, trailers)
                                .map_err(|error| CallError::trap(error.to_string()))?;
                        let invocation = store
                            .data()
                            .context
                            .invocation_id()
                            .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                        Ok(vec![Val::Tuple(vec![
                            scope(response.to_val(), invocation),
                            lift_future_plain(&mut store, transferred)
                                .map_err(|error| CallError::trap(error.to_string()))?,
                        ])])
                    })
                };
                let outcome =
                    trampoline::gate(&mut store, TYPES, "[static]response.new", args, real)
                        .await
                        .map_err(wasmtime::Error::new)?;
                let [Val::Tuple(values)] = outcome.as_slice() else {
                    return Err(wasmtime::Error::new(shape("resource and future")));
                };
                let [response, transferred] = values.as_slice() else {
                    return Err(wasmtime::Error::new(shape("resource and future")));
                };
                let response = Resource::<Response>::from_val(response.clone())
                    .map_err(wasmtime::Error::new)?;
                let transferred: FutureReader<TransferResult> =
                    lower_future_plain(&mut store, transferred.clone())?;
                Ok(((response, transferred),))
            })
        },
    )?;
    Ok(())
}

fn add_response_consume(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    use wasmtime_wasi_http::p3::bindings::http::types::HostResponseWithStore;
    use wasmtime_wasi_http::{WasiHttp, WasiHttpView};

    linker.instance(TYPES)?.func_wrap_async(
        "[static]response.consume-body",
        |mut store, (response, transferred): (Resource<Response>, FutureReader<TransferResult>)| {
            Box::new(async move {
                let invocation = store
                    .data()
                    .context
                    .invocation_id()
                    .ok_or_else(|| wasmtime::Error::msg("WASI call has no invocation id"))?;
                let args = vec![
                    scope(response.to_val(), invocation),
                    lift_future_plain(&mut store, transferred)?,
                ];
                let real: Real = |mut store, args| {
                    Box::pin(async move {
                        let [response, transferred] = <[Val; 2]>::try_from(args)
                            .map_err(|_| shape("response.consume-body arguments"))?;
                        let Val::Resource(resource) = &response else {
                            return Err(shape(RESPONSE_NAME));
                        };
                        validate_owned::<Response>(resource, store.data_mut())?;
                        let response = Resource::<Response>::from_val(response)?;
                        let transferred = lower_future_plain(&mut store, transferred)
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        let access = Access::<StoreData, WasiHttp>::new(
                            store.as_context_mut(),
                            WasiHttpView::http,
                        );
                        let (body, trailers) =
                            HostResponseWithStore::consume_body(access, response, transferred)
                                .map_err(|error| CallError::trap(error.to_string()))?;
                        Ok(vec![Val::Tuple(vec![
                            lift_stream_plain(&mut store, body)
                                .map_err(|error| CallError::trap(error.to_string()))?,
                            lift_future_plain(&mut store, trailers)
                                .map_err(|error| CallError::trap(error.to_string()))?,
                        ])])
                    })
                };
                let outcome = trampoline::gate(
                    &mut store,
                    TYPES,
                    "[static]response.consume-body",
                    args,
                    real,
                )
                .await
                .map_err(wasmtime::Error::new)?;
                let [Val::Tuple(values)] = outcome.as_slice() else {
                    return Err(wasmtime::Error::new(shape("body and trailers")));
                };
                let [body, trailers] = values.as_slice() else {
                    return Err(wasmtime::Error::new(shape("body and trailers")));
                };
                let body = lower_stream_plain(&mut store, body.clone())?;
                let trailers: FutureReader<TrailersResult> =
                    lower_future_plain(&mut store, trailers.clone())?;
                Ok(((body, trailers),))
            })
        },
    )?;
    Ok(())
}

fn add_send(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    use wasm_junction_core::WasiSettings;
    use wasmtime_wasi_http::p3::bindings::http::client::HostWithStore;
    use wasmtime_wasi_http::{WasiHttp, WasiHttpView};

    linker
        .instance("wasi:http/client@0.3.0")?
        .func_wrap_concurrent("send", |accessor, (request,): (Resource<Request>,)| {
            Box::pin(async move {
                let (invocation, method, scheme, authority, path, headers) =
                    accessor.with(|mut access| -> wasmtime::Result<_> {
                        let store = access.get();
                        let invocation = store.context.invocation_id().ok_or_else(|| {
                            wasmtime::Error::msg("WASI call has no invocation id")
                        })?;
                        let (method, scheme, authority, path, headers) =
                            request_context(store, &request)?;
                        Ok((invocation, method, scheme, authority, path, headers))
                    })?;
                let args = scope_values(
                    vec![
                        request.to_val(),
                        method.to_val(),
                        scheme.to_val(),
                        authority.to_val(),
                        path.to_val(),
                        headers.to_val(),
                    ],
                    invocation,
                );
                let real: RealConcurrent = |accessor, args| {
                    Box::pin(async move {
                        let [request, method, scheme, authority, path, headers] =
                            <[Val; 6]>::try_from(args).map_err(|_| shape("send context"))?;
                        validate_owned_arg::<Request>(&request, accessor)?;
                        let request = Resource::<Request>::from_val(request)?;
                        let method = Method::from_val(method)?;
                        let scheme = Option::<Scheme>::from_val(scheme)?;
                        let authority = Option::<String>::from_val(authority)?;
                        let path = Option::<String>::from_val(path)?;
                        let headers = Headers::from_val(headers)?;
                        let enabled = accessor.with(|mut access| {
                            access
                                .get()
                                .context
                                .settings()
                                .get::<WasiSettings>()
                                .is_some_and(WasiSettings::network_enabled)
                        });
                        if !enabled {
                            return Err(CallError::refused("outgoing HTTP is disabled"));
                        }
                        accessor
                            .with(|mut access| {
                                apply_request_context(
                                    access.get(),
                                    &request,
                                    method,
                                    scheme,
                                    authority,
                                    path,
                                    headers,
                                )
                            })
                            .map_err(|error| CallError::trap(error.to_string()))?;
                        let view = accessor.with_getter::<WasiHttp>(WasiHttpView::http);
                        let result = HostWithStore::send(&view, request).await;
                        let result = convert_trappable(result)?;
                        let invocation = accessor
                            .with(|mut access| access.get().context.invocation_id())
                            .ok_or_else(|| CallError::trap("WASI call has no invocation id"))?;
                        Ok(scope_values(vec![p3_result_value(result)], invocation))
                    })
                };
                let outcome = trampoline::gate_concurrent(
                    accessor,
                    "wasi:http/client@0.3.0",
                    "send",
                    args,
                    real,
                )
                .await;
                if let Ok(values) = &outcome
                    && let [Val::Result(Ok(Some(response)))] = values.as_slice()
                {
                    validate_owned_arg::<Response>(response, accessor)
                        .map_err(wasmtime::Error::new)?;
                }
                let result = finish_p3_error(
                    outcome,
                    ErrorCode::HttpRequestDenied,
                    decode_p3_result::<Resource<Response>, ErrorCode>,
                )?;
                Ok((result,))
            })
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_variants_round_trip() {
        assert!(matches!(
            Method::from_val(Method::Patch.to_val()),
            Ok(Method::Patch)
        ));
        assert!(matches!(
            Scheme::from_val(Scheme::Https.to_val()),
            Ok(Scheme::Https)
        ));
        assert!(matches!(
            HeaderError::from_val(HeaderError::Forbidden.to_val()),
            Ok(HeaderError::Forbidden)
        ));
    }

    #[test]
    fn http_error_codes_have_wit_case_names() {
        let Val::Variant { case, value: None } = ErrorCode::HttpRequestDenied.to_val() else {
            panic!("HTTP request denial changed shape");
        };
        assert_eq!(case, "HTTP-request-denied");
        let error = ErrorCode::DnsError(DnsErrorPayload {
            rcode: Some("refused".to_owned()),
            info_code: Some(5),
        });
        let ErrorCode::DnsError(decoded) = ErrorCode::from_val(error.to_val()).unwrap() else {
            panic!("DNS error changed case");
        };
        assert_eq!(decoded.rcode.as_deref(), Some("refused"));
        assert_eq!(decoded.info_code, Some(5));
        let header = ErrorCode::HttpRequestHeaderSize(Some(FieldSizePayload {
            field_name: Some("authorization".to_owned()),
            field_size: Some(99),
        }));
        assert!(matches!(
            ErrorCode::from_val(header.to_val()).unwrap(),
            ErrorCode::HttpRequestHeaderSize(Some(FieldSizePayload {
                field_size: Some(99),
                ..
            }))
        ));
        let body = ErrorCode::HttpResponseBodySize(Some(4_096));
        assert!(matches!(
            ErrorCode::from_val(body.to_val()).unwrap(),
            ErrorCode::HttpResponseBodySize(Some(4_096))
        ));
    }
}
