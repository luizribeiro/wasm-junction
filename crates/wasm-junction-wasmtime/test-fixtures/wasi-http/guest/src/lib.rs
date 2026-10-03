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
        match send(authority, path).await {
            Ok(response) => response,
            Err(ErrorCode::HttpRequestDenied) => "denied".into(),
            Err(error) => format!("error: {error:?}"),
        }
    }
}

async fn send(authority: String, path: String) -> Result<String, ErrorCode> {
    let headers = Fields::from_list(&[("x-client".into(), b"visible".to_vec())])
        .map_err(|_| ErrorCode::InternalError(Some("invalid headers".into())))?;
    let (mut body_writer, body_reader) = wit_stream::new();
    let (trailers_writer, trailers_reader) = wit_future::new(|| Ok(None));
    let (request, completion) = Request::new(headers, Some(body_reader), trailers_reader, None);
    request.set_method(&Method::Post).map_err(|()| ErrorCode::HttpRequestMethodInvalid)?;
    request.set_scheme(Some(&Scheme::Http)).map_err(|()| ErrorCode::HttpRequestUriInvalid)?;
    request.set_authority(Some(&authority)).map_err(|()| ErrorCode::HttpRequestUriInvalid)?;
    request.set_path_with_query(Some(&path)).map_err(|()| ErrorCode::HttpRequestUriInvalid)?;

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
