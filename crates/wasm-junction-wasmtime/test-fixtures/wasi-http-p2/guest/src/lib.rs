wit_bindgen::generate!({
    path: "../wit",
    world: "fixture",
});

struct Component;

impl exports::test::wasi_http_p2::probe::Guest for Component {
    fn request(authority: String, path: String) -> String {
        request(authority, path, false, None)
    }

    fn request_with_options(
        authority: String,
        path: String,
        secure: bool,
        timeout_nanoseconds: Option<u64>,
    ) -> String {
        request(authority, path, secure, timeout_nanoseconds)
    }

    fn drop_response(authority: String, path: String) -> String {
        match open(authority, path, false, None) {
            Ok(response) => {
                drop(response);
                "dropped".into()
            }
            Err(error) => format!("error: {error:?}"),
        }
    }

    fn large_request(authority: String, path: String) -> String {
        match send_large(authority, path) {
            Ok(()) => "sent".into(),
            Err(error) => format!("error: {error:?}"),
        }
    }
}

fn request(
    authority: String,
    path: String,
    secure: bool,
    timeout_nanoseconds: Option<u64>,
) -> String {
    match send(authority, path, secure, timeout_nanoseconds) {
        Ok(response) => response,
        Err(wasi::http::types::ErrorCode::HttpRequestDenied) => "denied".into(),
        Err(error) => format!("error: {error:?}"),
    }
}

fn error(message: &str) -> wasi::http::types::ErrorCode {
    wasi::http::types::ErrorCode::InternalError(Some(message.into()))
}

fn send(
    authority: String,
    path: String,
    secure: bool,
    timeout_nanoseconds: Option<u64>,
) -> Result<String, wasi::http::types::ErrorCode> {
    use wasi::http::types::IncomingBody;

    let response = open(authority, path, secure, timeout_nanoseconds)?;
    let status = response.status();
    drop(response.headers());
    let body = response.consume().map_err(|()| error("response body"))?;
    let stream = body.stream().map_err(|()| error("response stream"))?;
    let mut bytes = Vec::new();
    loop {
        match stream.blocking_read(4096) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(wasi::io::streams::StreamError::Closed) => break,
            Err(_) => return Err(error("read")),
        }
    }
    drop(stream);
    let trailers = IncomingBody::finish(body);
    trailers.subscribe().block();
    let _ = trailers.get();
    Ok(format!("{status}:{}", String::from_utf8_lossy(&bytes)))
}

fn open(
    authority: String,
    path: String,
    secure: bool,
    timeout_nanoseconds: Option<u64>,
) -> Result<wasi::http::types::IncomingResponse, wasi::http::types::ErrorCode> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{
        Fields, Method, OutgoingBody, OutgoingRequest, RequestOptions, Scheme,
    };

    let headers = Fields::from_list(&[("x-client".into(), b"visible".to_vec())])
        .map_err(|_| error("headers"))?;
    let request = OutgoingRequest::new(headers);
    request
        .set_method(&Method::Post)
        .map_err(|()| error("method"))?;
    let scheme = if secure { Scheme::Https } else { Scheme::Http };
    request
        .set_scheme(Some(&scheme))
        .map_err(|()| error("scheme"))?;
    request
        .set_authority(Some(&authority))
        .map_err(|()| error("authority"))?;
    request
        .set_path_with_query(Some(&path))
        .map_err(|()| error("path"))?;
    let body = request.body().map_err(|()| error("body"))?;
    let options = timeout_nanoseconds.map(|timeout| {
        let options = RequestOptions::new();
        options.set_connect_timeout(Some(timeout)).unwrap();
        options.set_first_byte_timeout(Some(timeout)).unwrap();
        options.set_between_bytes_timeout(Some(timeout)).unwrap();
        options
    });
    let future = outgoing_handler::handle(request, options)?;
    let stream = body.write().map_err(|()| error("stream"))?;
    stream
        .blocking_write_and_flush(b"guest-body")
        .map_err(|_| error("write"))?;
    drop(stream);
    OutgoingBody::finish(body, None)?;

    future.subscribe().block();
    future
        .get()
        .ok_or_else(|| error("not ready"))?
        .map_err(|()| error("consumed"))?
}

fn send_large(
    authority: String,
    path: String,
) -> Result<(), wasi::http::types::ErrorCode> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};

    const BODY_SIZE: usize = 512 * 1024;
    const CHUNK_SIZE: usize = 8 * 1024;
    const CHUNK: &[u8] = &[b'x'; CHUNK_SIZE];

    let headers =
        Fields::from_list(&[("content-length".into(), BODY_SIZE.to_string().into_bytes())])
            .map_err(|_| error("headers"))?;
    let request = OutgoingRequest::new(headers);
    request
        .set_method(&Method::Post)
        .map_err(|()| error("method"))?;
    request
        .set_scheme(Some(&Scheme::Http))
        .map_err(|()| error("scheme"))?;
    request
        .set_authority(Some(&authority))
        .map_err(|()| error("authority"))?;
    request
        .set_path_with_query(Some(&path))
        .map_err(|()| error("path"))?;
    let body = request.body().map_err(|()| error("body"))?;
    let future = outgoing_handler::handle(request, None)?;
    let stream = body.write().map_err(|()| error("stream"))?;

    write_nonblocking(&stream, CHUNK)?;
    future.subscribe().block();
    let response = future
        .get()
        .ok_or_else(|| error("not ready"))?
        .map_err(|()| error("consumed"))??;
    let response_body = response.consume().map_err(|()| error("response body"))?;
    drop(response_body);

    for _ in 1..BODY_SIZE / CHUNK_SIZE {
        write_nonblocking(&stream, CHUNK)?;
    }
    stream.flush().map_err(|_| error("flush"))?;
    {
        let completion = stream.subscribe();
        completion.block();
    }
    stream.check_write().map_err(|_| error("completion"))?;
    drop(stream);
    OutgoingBody::finish(body, None)
}

fn write_nonblocking(
    stream: &wasi::io::streams::OutputStream,
    mut bytes: &[u8],
) -> Result<(), wasi::http::types::ErrorCode> {
    while !bytes.is_empty() {
        let permit = stream.check_write().map_err(|_| error("write"))? as usize;
        if permit == 0 {
            let ready = stream.subscribe();
            ready.block();
            continue;
        }
        let count = permit.min(bytes.len());
        stream
            .write(&bytes[..count])
            .map_err(|_| error("write"))?;
        bytes = &bytes[count..];
    }
    Ok(())
}

export!(Component);
