wit_bindgen::generate!({
    path: "../wit",
    world: "fixture",
});

use wasip3::http::client;
use wasip3::http::types::{ErrorCode, Fields, Method, Request, Response, Scheme};
use wasip3::{wit_future, wit_stream};

struct Component;

impl exports::test::wasi_http::probe::Guest for Component {
    async fn request(authority: String, path: String) -> String {
        request(authority, path, false, None).await
    }

    async fn request_with_options(
        authority: String,
        path: String,
        secure: bool,
        timeout_nanoseconds: Option<u64>,
    ) -> String {
        request(authority, path, secure, timeout_nanoseconds).await
    }

    async fn empty_request(
        authority: String,
        path: String,
        timeout_nanoseconds: Option<u64>,
    ) -> String {
        match send_empty(authority, path, timeout_nanoseconds).await {
            Ok(response) => response,
            Err(ErrorCode::HttpRequestDenied) => "denied".into(),
            Err(error) => format!("error: {error:?}"),
        }
    }

    async fn drop_response(authority: String, path: String) -> String {
        match open_empty(authority, path, None).await {
            Ok(response) => {
                drop(response);
                "dropped".into()
            }
            Err(error) => format!("error: {error:?}"),
        }
    }

    async fn leak_response(authority: String, path: String) -> String {
        match open_empty(authority, path, None).await {
            Ok(response) => {
                std::mem::forget(response);
                "leaked".into()
            }
            Err(error) => format!("error: {error:?}"),
        }
    }

    async fn hold_response(authority: String, path: String) -> String {
        match open_empty(authority, path, None).await {
            Ok(response) => {
                wasip3::clocks::monotonic_clock::wait_for(60_000_000_000).await;
                drop(response);
                "held".into()
            }
            Err(error) => format!("error: {error:?}"),
        }
    }

    async fn large_request(authority: String, path: String) -> String {
        match send_large(authority, path).await {
            Ok(()) => "sent".into(),
            Err(error) => format!("error: {error:?}"),
        }
    }

    async fn coverage() -> String {
        match exercise_types().await {
            Ok(()) => "ok".into(),
            Err(error) => format!("error: {error:?}"),
        }
    }
}

async fn request(
    authority: String,
    path: String,
    secure: bool,
    timeout_nanoseconds: Option<u64>,
) -> String {
    match send(authority, path, secure, timeout_nanoseconds).await {
        Ok(response) => response,
        Err(ErrorCode::HttpRequestDenied) => "denied".into(),
        Err(error) => format!("error: {error:?}"),
    }
}

async fn exercise_types() -> Result<(), ErrorCode> {
    exercise_fields()?;
    exercise_options()?;
    exercise_request().await?;
    exercise_response().await
}

fn exercise_fields() -> Result<(), ErrorCode> {
    let fields = Fields::new();
    fields.set("x-one", &[b"a".to_vec()]).map_err(header)?;
    fields.append("x-one", b"b").map_err(header)?;
    let _ = fields.get("x-one");
    let _ = fields.has("x-one");
    let _ = fields.copy_all();
    drop(fields.clone());
    let _ = fields.get_and_delete("x-one").map_err(header)?;
    fields.set("x-two", &[b"c".to_vec()]).map_err(header)?;
    fields.delete("x-two").map_err(header)
}

fn exercise_options() -> Result<(), ErrorCode> {
    let options = wasip3::http::types::RequestOptions::new();
    options.set_connect_timeout(Some(1)).map_err(option)?;
    options.set_first_byte_timeout(Some(2)).map_err(option)?;
    options.set_between_bytes_timeout(Some(3)).map_err(option)?;
    let _ = options.get_connect_timeout();
    let _ = options.get_first_byte_timeout();
    let _ = options.get_between_bytes_timeout();
    drop(options.clone());
    Ok(())
}

async fn exercise_request() -> Result<(), ErrorCode> {
    let (unused_trailers, trailers) = wit_future::new(|| Ok(None));
    drop(unused_trailers);
    let (unused, completion) = Request::new(Fields::new(), None, trailers, None);
    drop((unused, completion));
    let (trailers_writer, trailers_reader) = wit_future::new(|| Ok(None));
    drop(trailers_writer);
    let (request, completion) = Request::new(Fields::new(), None, trailers_reader, None);
    let _ = request.get_method();
    let _ = request.get_path_with_query();
    let _ = request.get_scheme();
    let _ = request.get_authority();
    let _ = request.get_options();
    drop(request.get_headers());
    request.set_method(&Method::Patch).map_err(invalid)?;
    request
        .set_path_with_query(Some("/coverage"))
        .map_err(invalid)?;
    request.set_scheme(Some(&Scheme::Http)).map_err(invalid)?;
    request.set_authority(Some("localhost")).map_err(invalid)?;
    consume_request(request, completion).await
}

async fn consume_request(
    request: Request,
    completion: wasip3::wit_bindgen::rt::async_support::FutureReader<Result<(), ErrorCode>>,
) -> Result<(), ErrorCode> {
    let (result_writer, result_reader) = wit_future::new(|| Ok(()));
    drop(result_writer);
    let (body, trailers) = Request::consume_body(request, result_reader);
    drop((body, trailers, completion));
    Ok(())
}

async fn exercise_response() -> Result<(), ErrorCode> {
    let (unused_trailers, trailers) = wit_future::new(|| Ok(None));
    drop(unused_trailers);
    let (unused, completion) = Response::new(Fields::new(), None, trailers);
    drop((unused, completion));
    let (trailers_writer, trailers_reader) = wit_future::new(|| Ok(None));
    drop(trailers_writer);
    let (response, completion) = Response::new(Fields::new(), None, trailers_reader);
    let _ = response.get_status_code();
    response.set_status_code(204).map_err(invalid)?;
    drop(response.get_headers());
    let (result_writer, result_reader) = wit_future::new(|| Ok(()));
    drop(result_writer);
    let (body, trailers) = Response::consume_body(response, result_reader);
    drop((body, trailers, completion));
    Ok(())
}

fn invalid((): ()) -> ErrorCode {
    ErrorCode::InternalError(Some("invalid metadata".into()))
}

fn header(_: wasip3::http::types::HeaderError) -> ErrorCode {
    ErrorCode::InternalError(Some("invalid header".into()))
}

fn option(_: wasip3::http::types::RequestOptionsError) -> ErrorCode {
    ErrorCode::InternalError(Some("invalid option".into()))
}

async fn send(
    authority: String,
    path: String,
    secure: bool,
    timeout_nanoseconds: Option<u64>,
) -> Result<String, ErrorCode> {
    let headers = Fields::from_list(&[("x-client".into(), b"visible".to_vec())])
        .map_err(|_| ErrorCode::InternalError(Some("invalid headers".into())))?;
    let (mut body_writer, body_reader) = wit_stream::new();
    let (trailers_writer, trailers_reader) = wit_future::new(|| Ok(None));
    let options = timeout_nanoseconds
        .map(|timeout| {
            let options = wasip3::http::types::RequestOptions::new();
            options.set_connect_timeout(Some(timeout)).map_err(option)?;
            options
                .set_first_byte_timeout(Some(timeout))
                .map_err(option)?;
            options
                .set_between_bytes_timeout(Some(timeout))
                .map_err(option)?;
            Ok(options)
        })
        .transpose()?;
    let (request, completion) = Request::new(headers, Some(body_reader), trailers_reader, options);
    request
        .set_method(&Method::Post)
        .map_err(|()| ErrorCode::HttpRequestMethodInvalid)?;
    let scheme = if secure { Scheme::Https } else { Scheme::Http };
    request
        .set_scheme(Some(&scheme))
        .map_err(|()| ErrorCode::HttpRequestUriInvalid)?;
    request
        .set_authority(Some(&authority))
        .map_err(|()| ErrorCode::HttpRequestUriInvalid)?;
    request
        .set_path_with_query(Some(&path))
        .map_err(|()| ErrorCode::HttpRequestUriInvalid)?;

    wasip3::wit_bindgen::spawn_local(async move {
        let unwritten = body_writer.write_all(b"guest-body".to_vec()).await;
        assert!(unwritten.is_empty());
        drop(body_writer);
        drop(trailers_writer);
    });
    let response = client::send(request).await?;
    completion.await?;
    receive(response).await
}

async fn send_empty(
    authority: String,
    path: String,
    timeout_nanoseconds: Option<u64>,
) -> Result<String, ErrorCode> {
    let response = open_empty(authority, path, timeout_nanoseconds).await?;
    receive(response).await
}

async fn send_large(authority: String, path: String) -> Result<(), ErrorCode> {
    const BODY_SIZE: usize = 512 * 1024;
    const CHUNK_SIZE: usize = 8 * 1024;

    let headers =
        Fields::from_list(&[("content-length".into(), BODY_SIZE.to_string().into_bytes())])
            .map_err(|_| ErrorCode::InternalError(Some("invalid headers".into())))?;
    let (mut body_writer, body_reader) = wit_stream::new();
    let (trailers_writer, trailers_reader) = wit_future::new(|| Ok(None));
    let (request, completion) = Request::new(headers, Some(body_reader), trailers_reader, None);
    request
        .set_method(&Method::Post)
        .map_err(|()| ErrorCode::HttpRequestMethodInvalid)?;
    request
        .set_scheme(Some(&Scheme::Http))
        .map_err(|()| ErrorCode::HttpRequestUriInvalid)?;
    request
        .set_authority(Some(&authority))
        .map_err(|()| ErrorCode::HttpRequestUriInvalid)?;
    request
        .set_path_with_query(Some(&path))
        .map_err(|()| ErrorCode::HttpRequestUriInvalid)?;

    wasip3::wit_bindgen::spawn_local(async move {
        for _ in 0..BODY_SIZE / CHUNK_SIZE {
            let unwritten = body_writer.write_all(vec![b'x'; CHUNK_SIZE]).await;
            assert!(unwritten.is_empty());
            wasip3::clocks::monotonic_clock::wait_for(1_000_000).await;
        }
        drop(body_writer);
        drop(trailers_writer);
    });
    let response = client::send(request).await?;
    drop(response);
    completion.await
}

async fn open_empty(
    authority: String,
    path: String,
    timeout_nanoseconds: Option<u64>,
) -> Result<Response, ErrorCode> {
    let options = timeout_nanoseconds
        .map(|timeout| {
            let options = wasip3::http::types::RequestOptions::new();
            options.set_connect_timeout(Some(timeout)).map_err(option)?;
            options
                .set_first_byte_timeout(Some(timeout))
                .map_err(option)?;
            options
                .set_between_bytes_timeout(Some(timeout))
                .map_err(option)?;
            Ok(options)
        })
        .transpose()?;
    let (trailers, trailers_reader) = wit_future::new(|| Ok(None));
    let (request, completion) = Request::new(Fields::new(), None, trailers_reader, options);
    request
        .set_method(&Method::Get)
        .map_err(|()| ErrorCode::HttpRequestMethodInvalid)?;
    request
        .set_scheme(Some(&Scheme::Http))
        .map_err(|()| ErrorCode::HttpRequestUriInvalid)?;
    request
        .set_authority(Some(&authority))
        .map_err(|()| ErrorCode::HttpRequestUriInvalid)?;
    request
        .set_path_with_query(Some(&path))
        .map_err(|()| ErrorCode::HttpRequestUriInvalid)?;
    drop(trailers);
    let response = client::send(request).await?;
    completion.await?;
    Ok(response)
}

async fn receive(response: Response) -> Result<String, ErrorCode> {
    let status = response.get_status_code();
    let (result_writer, result_reader) = wit_future::new(|| Ok(()));
    let (body, trailers) = Response::consume_body(response, result_reader);
    drop(result_writer);
    let bytes = body.collect().await;
    trailers.await?;
    Ok(format!("{status}:{}", String::from_utf8_lossy(&bytes)))
}

export!(Component);
