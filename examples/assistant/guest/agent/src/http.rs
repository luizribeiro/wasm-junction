use http_error::{message as http_error, plain};
use wasip3::http::client;
use wasip3::http::types::{Fields, Method, Request, Response, Scheme};
use wasip3::{wit_future, wit_stream};

pub(crate) async fn post(
    scheme: &str,
    authority: &str,
    path_prefix: &str,
    api_key: &str,
    body: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let scheme = match scheme {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        _ => return Err("model URL must use HTTP or HTTPS".into()),
    };
    let path = format!("{path_prefix}/chat/completions");
    let headers = Fields::new();
    headers
        .append("content-type", b"application/json")
        .map_err(|error| format!("invalid content-type header: {}", plain(&error)))?;
    headers
        .append("authorization", format!("Bearer {api_key}").as_bytes())
        .map_err(|error| format!("invalid authorization header: {}", plain(&error)))?;
    let (mut body_writer, body_reader) = wit_stream::new();
    let (trailers_writer, trailers_reader) = wit_future::new(|| Ok(None));
    let (request, completion) = Request::new(headers, Some(body_reader), trailers_reader, None);
    request.set_method(&Method::Post).map_err(invalid)?;
    request.set_scheme(Some(&scheme)).map_err(invalid)?;
    request.set_authority(Some(authority)).map_err(invalid)?;
    request.set_path_with_query(Some(&path)).map_err(invalid)?;
    wit_bindgen::spawn_local(async move {
        let _ = body_writer.write_all(body).await;
        drop((body_writer, trailers_writer));
    });
    let response = client::send(request)
        .await
        .map_err(|error| format!("model request failed: {}", http_error(&error)))?;
    completion
        .await
        .map_err(|error| format!("model request body failed: {}", http_error(&error)))?;
    receive(response).await
}

async fn receive(response: Response) -> Result<Vec<u8>, String> {
    let status = response.get_status_code();
    let (completion, completion_reader) = wit_future::new(|| Ok(()));
    let (body, trailers) = Response::consume_body(response, completion_reader);
    let bytes = body.collect().await;
    trailers
        .await
        .map_err(|error| format!("model response failed: {}", http_error(&error)))?;
    drop(completion);
    if !(200..300).contains(&status) {
        return Err(format!(
            "model returned HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    Ok(bytes)
}

fn invalid((): ()) -> String {
    "invalid model request URL".into()
}
