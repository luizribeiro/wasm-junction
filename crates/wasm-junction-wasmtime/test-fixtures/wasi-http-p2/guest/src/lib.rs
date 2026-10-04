wit_bindgen::generate!({
    path: "../wit",
    world: "fixture",
});

struct Component;

impl exports::test::wasi_http_p2::probe::Guest for Component {
    fn request(authority: String, path: String) -> String {
        match send(authority, path) {
            Ok(response) => response,
            Err(wasi::http::types::ErrorCode::HttpRequestDenied) => "denied".into(),
            Err(error) => format!("error: {error:?}"),
        }
    }
}

fn error(message: &str) -> wasi::http::types::ErrorCode {
    wasi::http::types::ErrorCode::InternalError(Some(message.into()))
}

fn send(authority: String, path: String) -> Result<String, wasi::http::types::ErrorCode> {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, IncomingBody, Method, OutgoingBody, OutgoingRequest, Scheme};

    let headers = Fields::from_list(&[("x-client".into(), b"visible".to_vec())])
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
    stream
        .blocking_write_and_flush(b"guest-body")
        .map_err(|_| error("write"))?;
    drop(stream);
    OutgoingBody::finish(body, None)?;

    future.subscribe().block();
    let response = future
        .get()
        .ok_or_else(|| error("not ready"))?
        .map_err(|()| error("consumed"))??;
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

export!(Component);
