wit_bindgen::generate!({
    path: "../../wit",
    world: "plugin",
});

use wasip3::http::client;
use wasip3::http::types::{ErrorCode, Fields, Method, Request, Response, Scheme};
use wasip3::{wit_future, wit_stream};

struct Component;

impl exports::example::http::client::Guest for Component {
    async fn fetch(authority: String, path: String) -> String {
        match fetch(authority, path).await {
            Ok(response) => response,
            Err(ErrorCode::HttpRequestDenied) => "denied".into(),
            Err(error) => format!("error: {error:?}"),
        }
    }
}

async fn fetch(authority: String, path: String) -> Result<String, ErrorCode> {
    let headers = Fields::from_list(&[("x-client".into(), b"visible".to_vec())])
        .map_err(|_| ErrorCode::InternalError(Some("invalid headers".into())))?;
    let (mut body_writer, body_reader) = wit_stream::new();
    let (trailers_writer, trailers_reader) = wit_future::new(|| Ok(None));
    let (request, completion) = Request::new(headers, Some(body_reader), trailers_reader, None);
    request.set_method(&Method::Post).map_err(invalid)?;
    request.set_scheme(Some(&Scheme::Http)).map_err(invalid)?;
    request.set_authority(Some(&authority)).map_err(invalid)?;
    request.set_path_with_query(Some(&path)).map_err(invalid)?;
    wasip3::wit_bindgen::spawn_local(async move {
        assert!(body_writer.write_all(b"example-body".to_vec()).await.is_empty());
        drop((body_writer, trailers_writer));
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

fn invalid((): ()) -> ErrorCode {
    ErrorCode::HttpRequestUriInvalid
}

export!(Component);
