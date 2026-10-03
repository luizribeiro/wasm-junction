use super::*;
use wasmtime::component::{Access, ComponentType, FutureReader, StreamReader};
use wasmtime::{AsContextMut, StoreContextMut};
use wasmtime_wasi_http::p3::bindings::http::types::{
    DnsErrorPayload, ErrorCode, FieldSizePayload, Fields, HeaderError, Method, Request,
    RequestOptions, RequestOptionsError, Response, Scheme, TlsAlertReceivedPayload,
};

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

type TransferResult = Result<(), ErrorCode>;
type TrailersResult = Result<Option<Resource<Fields>>, ErrorCode>;

fn drop_request_options(
    store: &mut StoreData,
    options: Resource<RequestOptions>,
) -> wasmtime::Result<()> {
    let mut view = views::http(store);
    wasmtime_wasi_http::p3::bindings::http::types::HostRequestOptions::drop(&mut view, options)
}

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
    Ok(())
}

fn add_request_new(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    use wasmtime_wasi_http::p3::bindings::http::types::HostRequestWithStore;
    use wasmtime_wasi_http::{WasiHttp, WasiHttpView};

    linker.instance(TYPES)?.func_wrap_async(
        "[static]request.new",
        |mut store,
         (headers, contents, trailers, options): (
            Resource<Fields>,
            Option<StreamReader<u8>>,
            FutureReader<TrailersResult>,
            Option<Resource<RequestOptions>>,
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

fn variant(case: &str, value: Option<Val>) -> Val {
    Val::Variant {
        case: case.to_owned(),
        value: value.map(Box::new),
    }
}

impl ToVal for Method {
    fn to_val(self) -> Val {
        let (case, value) = match self {
            Self::Get => ("get", None),
            Self::Head => ("head", None),
            Self::Post => ("post", None),
            Self::Put => ("put", None),
            Self::Delete => ("delete", None),
            Self::Connect => ("connect", None),
            Self::Options => ("options", None),
            Self::Trace => ("trace", None),
            Self::Patch => ("patch", None),
            Self::Other(value) => ("other", Some(value.to_val())),
        };
        variant(case, value)
    }
}

impl FromVal for Method {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Variant { case, value } = value else {
            return Err(shape("method"));
        };
        Ok(match (case.as_str(), value) {
            ("get", None) => Self::Get,
            ("head", None) => Self::Head,
            ("post", None) => Self::Post,
            ("put", None) => Self::Put,
            ("delete", None) => Self::Delete,
            ("connect", None) => Self::Connect,
            ("options", None) => Self::Options,
            ("trace", None) => Self::Trace,
            ("patch", None) => Self::Patch,
            ("other", Some(value)) => Self::Other(String::from_val(*value)?),
            _ => return Err(shape("method case")),
        })
    }
}

impl ToVal for Scheme {
    fn to_val(self) -> Val {
        match self {
            Self::Http => variant("HTTP", None),
            Self::Https => variant("HTTPS", None),
            Self::Other(value) => variant("other", Some(value.to_val())),
        }
    }
}

impl FromVal for Scheme {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Variant { case, value } = value else {
            return Err(shape("scheme"));
        };
        match (case.as_str(), value) {
            ("HTTP", None) => Ok(Self::Http),
            ("HTTPS", None) => Ok(Self::Https),
            ("other", Some(value)) => String::from_val(*value).map(Self::Other),
            _ => Err(shape("scheme case")),
        }
    }
}

macro_rules! error_enum {
    ($ty:ty, $expected:literal, $($case:literal => $variant:ident),+ $(,)?) => {
        impl ToVal for $ty {
            fn to_val(self) -> Val {
                match self {
                    $(Self::$variant => variant($case, None),)+
                    Self::Other(value) => variant("other", Some(value.to_val())),
                }
            }
        }

        impl FromVal for $ty {
            fn from_val(value: Val) -> Result<Self, CallError> {
                let Val::Variant { case, value } = value else { return Err(shape($expected)); };
                match (case.as_str(), value) {
                    $(($case, None) => Ok(Self::$variant),)+
                    ("other", Some(value)) => Option::<String>::from_val(*value).map(Self::Other),
                    _ => Err(shape(concat!($expected, " case"))),
                }
            }
        }
    };
}

error_enum!(HeaderError, "header-error",
    "invalid-syntax" => InvalidSyntax, "forbidden" => Forbidden,
    "immutable" => Immutable, "size-exceeded" => SizeExceeded);
error_enum!(RequestOptionsError, "request-options-error",
    "not-supported" => NotSupported, "immutable" => Immutable);

fn record(fields: impl IntoIterator<Item = (&'static str, Val)>) -> Val {
    Val::Record(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn field_size_value(payload: FieldSizePayload) -> Val {
    record([
        ("field-name", payload.field_name.to_val()),
        ("field-size", payload.field_size.to_val()),
    ])
}

impl ToVal for ErrorCode {
    #[allow(clippy::too_many_lines, reason = "the WIT variant has 38 cases")]
    fn to_val(self) -> Val {
        let (case, value) = match self {
            Self::DnsTimeout => ("DNS-timeout", None),
            Self::DnsError(payload) => (
                "DNS-error",
                Some(record([
                    ("rcode", payload.rcode.to_val()),
                    ("info-code", payload.info_code.to_val()),
                ])),
            ),
            Self::DestinationNotFound => ("destination-not-found", None),
            Self::DestinationUnavailable => ("destination-unavailable", None),
            Self::DestinationIpProhibited => ("destination-IP-prohibited", None),
            Self::DestinationIpUnroutable => ("destination-IP-unroutable", None),
            Self::ConnectionRefused => ("connection-refused", None),
            Self::ConnectionTerminated => ("connection-terminated", None),
            Self::ConnectionTimeout => ("connection-timeout", None),
            Self::ConnectionReadTimeout => ("connection-read-timeout", None),
            Self::ConnectionWriteTimeout => ("connection-write-timeout", None),
            Self::ConnectionLimitReached => ("connection-limit-reached", None),
            Self::TlsProtocolError => ("TLS-protocol-error", None),
            Self::TlsCertificateError => ("TLS-certificate-error", None),
            Self::TlsAlertReceived(payload) => (
                "TLS-alert-received",
                Some(record([
                    ("alert-id", payload.alert_id.to_val()),
                    ("alert-message", payload.alert_message.to_val()),
                ])),
            ),
            Self::HttpRequestDenied => ("HTTP-request-denied", None),
            Self::HttpRequestLengthRequired => ("HTTP-request-length-required", None),
            Self::HttpRequestBodySize(payload) => {
                ("HTTP-request-body-size", Some(payload.to_val()))
            }
            Self::HttpRequestMethodInvalid => ("HTTP-request-method-invalid", None),
            Self::HttpRequestUriInvalid => ("HTTP-request-URI-invalid", None),
            Self::HttpRequestUriTooLong => ("HTTP-request-URI-too-long", None),
            Self::HttpRequestHeaderSectionSize(payload) => {
                ("HTTP-request-header-section-size", Some(payload.to_val()))
            }
            Self::HttpRequestHeaderSize(payload) => (
                "HTTP-request-header-size",
                Some(Val::Option(
                    payload.map(|payload| Box::new(field_size_value(payload))),
                )),
            ),
            Self::HttpRequestTrailerSectionSize(payload) => {
                ("HTTP-request-trailer-section-size", Some(payload.to_val()))
            }
            Self::HttpRequestTrailerSize(payload) => {
                ("HTTP-request-trailer-size", Some(field_size_value(payload)))
            }
            Self::HttpResponseIncomplete => ("HTTP-response-incomplete", None),
            Self::HttpResponseHeaderSectionSize(payload) => {
                ("HTTP-response-header-section-size", Some(payload.to_val()))
            }
            Self::HttpResponseHeaderSize(payload) => {
                ("HTTP-response-header-size", Some(field_size_value(payload)))
            }
            Self::HttpResponseBodySize(payload) => {
                ("HTTP-response-body-size", Some(payload.to_val()))
            }
            Self::HttpResponseTrailerSectionSize(payload) => {
                ("HTTP-response-trailer-section-size", Some(payload.to_val()))
            }
            Self::HttpResponseTrailerSize(payload) => (
                "HTTP-response-trailer-size",
                Some(field_size_value(payload)),
            ),
            Self::HttpResponseTransferCoding(payload) => {
                ("HTTP-response-transfer-coding", Some(payload.to_val()))
            }
            Self::HttpResponseContentCoding(payload) => {
                ("HTTP-response-content-coding", Some(payload.to_val()))
            }
            Self::HttpResponseTimeout => ("HTTP-response-timeout", None),
            Self::HttpUpgradeFailed => ("HTTP-upgrade-failed", None),
            Self::HttpProtocolError => ("HTTP-protocol-error", None),
            Self::LoopDetected => ("loop-detected", None),
            Self::ConfigurationError => ("configuration-error", None),
            Self::InternalError(payload) => ("internal-error", Some(payload.to_val())),
        };
        variant(case, value)
    }
}

fn payload<T: FromVal>(value: Option<Box<Val>>, expected: &str) -> Result<T, CallError> {
    value
        .ok_or_else(|| shape(expected))
        .and_then(|value| T::from_val(*value))
}

fn fields<const N: usize>(value: Val, expected: &str) -> Result<[Val; N], CallError> {
    let Val::Record(fields) = value else {
        return Err(shape(expected));
    };
    fields
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| shape(expected))
}

fn no_payload(value: &Option<Box<Val>>) -> Result<(), CallError> {
    if value.is_none() {
        Ok(())
    } else {
        Err(shape("payload-free error-code"))
    }
}

fn field_size_from_val(value: Val) -> Result<FieldSizePayload, CallError> {
    let [field_name, field_size] = fields(value, "field-size-payload")?;
    Ok(FieldSizePayload {
        field_name: Option::<String>::from_val(field_name)?,
        field_size: Option::<u32>::from_val(field_size)?,
    })
}

fn decode_transport_error(
    case: &str,
    value: Option<Box<Val>>,
) -> Option<Result<ErrorCode, CallError>> {
    let decoded = match case {
        "DNS-timeout" => no_payload(&value).map(|()| ErrorCode::DnsTimeout),
        "DNS-error" => (|| {
            let [rcode, info_code] = fields(
                *value.ok_or_else(|| shape("DNS-error payload"))?,
                "DNS-error payload",
            )?;
            Ok(ErrorCode::DnsError(DnsErrorPayload {
                rcode: Option::<String>::from_val(rcode)?,
                info_code: Option::<u16>::from_val(info_code)?,
            }))
        })(),
        "destination-not-found" => no_payload(&value).map(|()| ErrorCode::DestinationNotFound),
        "destination-unavailable" => no_payload(&value).map(|()| ErrorCode::DestinationUnavailable),
        "destination-IP-prohibited" => {
            no_payload(&value).map(|()| ErrorCode::DestinationIpProhibited)
        }
        "destination-IP-unroutable" => {
            no_payload(&value).map(|()| ErrorCode::DestinationIpUnroutable)
        }
        "connection-refused" => no_payload(&value).map(|()| ErrorCode::ConnectionRefused),
        "connection-terminated" => no_payload(&value).map(|()| ErrorCode::ConnectionTerminated),
        "connection-timeout" => no_payload(&value).map(|()| ErrorCode::ConnectionTimeout),
        "connection-read-timeout" => no_payload(&value).map(|()| ErrorCode::ConnectionReadTimeout),
        "connection-write-timeout" => {
            no_payload(&value).map(|()| ErrorCode::ConnectionWriteTimeout)
        }
        "connection-limit-reached" => {
            no_payload(&value).map(|()| ErrorCode::ConnectionLimitReached)
        }
        "TLS-protocol-error" => no_payload(&value).map(|()| ErrorCode::TlsProtocolError),
        "TLS-certificate-error" => no_payload(&value).map(|()| ErrorCode::TlsCertificateError),
        "TLS-alert-received" => (|| {
            let [alert_id, alert_message] = fields(
                *value.ok_or_else(|| shape("TLS alert payload"))?,
                "TLS alert payload",
            )?;
            Ok(ErrorCode::TlsAlertReceived(TlsAlertReceivedPayload {
                alert_id: Option::<u8>::from_val(alert_id)?,
                alert_message: Option::<String>::from_val(alert_message)?,
            }))
        })(),
        _ => return None,
    };
    Some(decoded)
}

fn decode_request_error(
    case: &str,
    value: Option<Box<Val>>,
) -> Option<Result<ErrorCode, CallError>> {
    let decoded = match case {
        "HTTP-request-denied" => no_payload(&value).map(|()| ErrorCode::HttpRequestDenied),
        "HTTP-request-length-required" => {
            no_payload(&value).map(|()| ErrorCode::HttpRequestLengthRequired)
        }
        "HTTP-request-body-size" => payload(value, "body size").map(ErrorCode::HttpRequestBodySize),
        "HTTP-request-method-invalid" => {
            no_payload(&value).map(|()| ErrorCode::HttpRequestMethodInvalid)
        }
        "HTTP-request-URI-invalid" => no_payload(&value).map(|()| ErrorCode::HttpRequestUriInvalid),
        "HTTP-request-URI-too-long" => {
            no_payload(&value).map(|()| ErrorCode::HttpRequestUriTooLong)
        }
        "HTTP-request-header-section-size" => {
            payload(value, "header section size").map(ErrorCode::HttpRequestHeaderSectionSize)
        }
        "HTTP-request-header-size" => (|| {
            let Val::Option(payload) = *value.ok_or_else(|| shape("header size"))? else {
                return Err(shape("optional header size"));
            };
            Ok(ErrorCode::HttpRequestHeaderSize(
                payload
                    .map(|value| field_size_from_val(*value))
                    .transpose()?,
            ))
        })(),
        "HTTP-request-trailer-section-size" => {
            payload(value, "trailer section size").map(ErrorCode::HttpRequestTrailerSectionSize)
        }
        "HTTP-request-trailer-size" => value
            .ok_or_else(|| shape("trailer size"))
            .and_then(|value| field_size_from_val(*value))
            .map(ErrorCode::HttpRequestTrailerSize),
        _ => return None,
    };
    Some(decoded)
}

fn decode_response_error(
    case: &str,
    value: Option<Box<Val>>,
) -> Option<Result<ErrorCode, CallError>> {
    let decoded = match case {
        "HTTP-response-incomplete" => {
            no_payload(&value).map(|()| ErrorCode::HttpResponseIncomplete)
        }
        "HTTP-response-header-section-size" => {
            payload(value, "header section size").map(ErrorCode::HttpResponseHeaderSectionSize)
        }
        "HTTP-response-header-size" => value
            .ok_or_else(|| shape("header size"))
            .and_then(|value| field_size_from_val(*value))
            .map(ErrorCode::HttpResponseHeaderSize),
        "HTTP-response-body-size" => {
            payload(value, "body size").map(ErrorCode::HttpResponseBodySize)
        }
        "HTTP-response-trailer-section-size" => {
            payload(value, "trailer section size").map(ErrorCode::HttpResponseTrailerSectionSize)
        }
        "HTTP-response-trailer-size" => value
            .ok_or_else(|| shape("trailer size"))
            .and_then(|value| field_size_from_val(*value))
            .map(ErrorCode::HttpResponseTrailerSize),
        "HTTP-response-transfer-coding" => {
            payload(value, "transfer coding").map(ErrorCode::HttpResponseTransferCoding)
        }
        "HTTP-response-content-coding" => {
            payload(value, "content coding").map(ErrorCode::HttpResponseContentCoding)
        }
        "HTTP-response-timeout" => no_payload(&value).map(|()| ErrorCode::HttpResponseTimeout),
        "HTTP-upgrade-failed" => no_payload(&value).map(|()| ErrorCode::HttpUpgradeFailed),
        "HTTP-protocol-error" => no_payload(&value).map(|()| ErrorCode::HttpProtocolError),
        "loop-detected" => no_payload(&value).map(|()| ErrorCode::LoopDetected),
        "configuration-error" => no_payload(&value).map(|()| ErrorCode::ConfigurationError),
        _ => return None,
    };
    Some(decoded)
}

impl FromVal for ErrorCode {
    fn from_val(value: Val) -> Result<Self, CallError> {
        let Val::Variant { case, value } = value else {
            return Err(shape("error-code"));
        };
        if let Some(decoded) = decode_transport_error(&case, value.clone()) {
            return decoded;
        }
        if let Some(decoded) = decode_request_error(&case, value.clone()) {
            return decoded;
        }
        if let Some(decoded) = decode_response_error(&case, value.clone()) {
            return decoded;
        }
        match case.as_str() {
            "internal-error" => payload(value, "internal error").map(Self::InternalError),
            _ => Err(shape("error-code case")),
        }
    }
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
        assert_eq!(
            ErrorCode::HttpRequestDenied.to_val(),
            variant("HTTP-request-denied", None)
        );
        assert_eq!(
            ErrorCode::HttpRequestHeaderSize(Some(FieldSizePayload {
                field_name: Some("authorization".to_owned()),
                field_size: Some(99),
            }))
            .to_val(),
            variant(
                "HTTP-request-header-size",
                Some(Val::Option(Some(Box::new(field_size_value(
                    FieldSizePayload {
                        field_name: Some("authorization".to_owned()),
                        field_size: Some(99),
                    }
                ))))),
            )
        );
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
