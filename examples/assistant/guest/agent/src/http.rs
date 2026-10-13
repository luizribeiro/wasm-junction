use wasip3::http::client;
use wasip3::http::types::{ErrorCode, Fields, Method, Request, Response, Scheme};
use wasip3::{wit_future, wit_stream};

pub async fn post(base_url: &str, api_key: &str, body: Vec<u8>) -> Result<Vec<u8>, String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let location = Location::parse(&url)?;
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
    request
        .set_scheme(Some(&location.scheme))
        .map_err(invalid)?;
    request
        .set_authority(Some(&location.authority))
        .map_err(invalid)?;
    request
        .set_path_with_query(Some(&location.path))
        .map_err(invalid)?;
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

struct Location {
    scheme: Scheme,
    authority: String,
    path: String,
}

impl Location {
    fn parse(url: &str) -> Result<Self, String> {
        let (scheme, rest) = url
            .split_once("://")
            .ok_or_else(|| "model URL has no scheme".to_owned())?;
        let scheme = match scheme {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err("model URL must use HTTP or HTTPS".into()),
        };
        let separator = rest.find(['/', '?']);
        let (authority, path) = separator.map_or((rest, "/"), |index| rest.split_at(index));
        if authority.is_empty() {
            return Err("model URL has no authority".into());
        }
        let path = if path.starts_with('?') {
            format!("/{path}")
        } else {
            path.to_owned()
        };
        Ok(Self {
            scheme,
            authority: authority.into(),
            path,
        })
    }
}

fn invalid((): ()) -> String {
    "invalid model request URL".into()
}

fn http_error(error: &ErrorCode) -> String {
    match error {
        ErrorCode::HttpRequestDenied => "request denied by policy".into(),
        ErrorCode::InternalError(Some(message)) => message.clone(),
        _ => plain(error),
    }
}

fn plain(value: &impl std::fmt::Debug) -> String {
    let debug = format!("{value:?}");
    let name = debug
        .strip_prefix("ErrorCode::")
        .unwrap_or(&debug)
        .split('(')
        .next()
        .unwrap_or(&debug);
    let characters = name.chars().collect::<Vec<_>>();
    let mut words = String::new();
    for (index, character) in characters.iter().copied().enumerate() {
        let previous = index.checked_sub(1).and_then(|at| characters.get(at));
        let next = characters.get(index + 1);
        if index > 0
            && character.is_uppercase()
            && (previous.is_some_and(|character| character.is_lowercase())
                || next.is_some_and(|character| character.is_lowercase())
                    && previous.is_some_and(|character| character.is_uppercase()))
        {
            words.push(' ');
        }
        words.extend(character.to_lowercase());
    }
    words
}
