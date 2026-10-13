wit_bindgen::generate!({
    path: "../../wit",
    world: "web-tools-component",
});

use http_error::{message as http_error, plain};
use wasip3::http::client;
use wasip3::http::types::{ErrorCode, Fields, Method, Request, RequestOptions, Response, Scheme};
use wasip3::wit_future;
use wit_bindgen::rt::async_support::StreamResult;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: u64 = 10_000_000_000;

struct Component;

impl exports::example::assistant::web_tools::Guest for Component {
    async fn fetch(url: String) -> Result<String, String> {
        fetch(&url).await.map_err(|error| http_error(&error))
    }
}

async fn fetch(url: &str) -> Result<String, ErrorCode> {
    let location = Location::parse(url)?;
    let (trailers, trailers_reader) = wit_future::new(|| Ok(None));
    let options = RequestOptions::new();
    options
        .set_connect_timeout(Some(REQUEST_TIMEOUT))
        .map_err(options_error)?;
    options
        .set_first_byte_timeout(Some(REQUEST_TIMEOUT))
        .map_err(options_error)?;
    options
        .set_between_bytes_timeout(Some(REQUEST_TIMEOUT))
        .map_err(options_error)?;
    let (request, completion) = Request::new(Fields::new(), None, trailers_reader, Some(options));
    request.set_method(&Method::Get).map_err(invalid)?;
    request
        .set_scheme(Some(&location.scheme))
        .map_err(invalid)?;
    request
        .set_authority(Some(&location.authority))
        .map_err(invalid)?;
    request
        .set_path_with_query(Some(&location.path))
        .map_err(invalid)?;
    drop(trailers);
    let response = client::send(request).await?;
    completion.await?;
    receive(response).await
}

async fn receive(response: Response) -> Result<String, ErrorCode> {
    let status = response.get_status_code();
    let (completion, completion_reader) = wit_future::new(|| Ok(()));
    let (mut body, trailers) = Response::consume_body(response, completion_reader);
    let mut bytes = Vec::with_capacity(MAX_RESPONSE_BYTES + 1);
    loop {
        let (status, next) = body.read(bytes).await;
        bytes = next;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(ErrorCode::InternalError(Some(
                "HTTP response exceeds 1 MiB".into(),
            )));
        }
        match status {
            StreamResult::Complete(_) => {}
            StreamResult::Dropped => break,
            StreamResult::Cancelled => {
                return Err(ErrorCode::InternalError(Some(
                    "HTTP response was cancelled".into(),
                )));
            }
        }
    }
    trailers.await?;
    drop(completion);
    if !(200..300).contains(&status) {
        return Err(ErrorCode::InternalError(Some(format!(
            "HTTP response status {status}"
        ))));
    }
    String::from_utf8(bytes).map_err(|error| ErrorCode::InternalError(Some(error.to_string())))
}

struct Location {
    scheme: Scheme,
    authority: String,
    path: String,
}

impl Location {
    fn parse(url: &str) -> Result<Self, ErrorCode> {
        let (scheme, rest) = url
            .split_once("://")
            .ok_or(ErrorCode::HttpRequestUriInvalid)?;
        let scheme = match scheme {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err(ErrorCode::HttpProtocolError),
        };
        let separator = rest.find(['/', '?']);
        let (authority, path) = separator.map_or((rest, "/"), |index| rest.split_at(index));
        let path = if path.starts_with('?') {
            format!("/{path}")
        } else {
            path.to_owned()
        };
        if authority.is_empty() {
            return Err(ErrorCode::HttpRequestUriInvalid);
        }
        Ok(Self {
            scheme,
            authority: authority.into(),
            path,
        })
    }
}

fn invalid((): ()) -> ErrorCode {
    ErrorCode::HttpRequestUriInvalid
}

fn options_error(error: wasip3::http::types::RequestOptionsError) -> ErrorCode {
    ErrorCode::InternalError(Some(format!(
        "request timeout is unavailable: {}",
        plain(&error)
    )))
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, Location, Scheme};

    #[test]
    fn parses_query_without_path() {
        let location = Location::parse("http://example.test?x=1").unwrap();
        assert_eq!(location.authority, "example.test");
        assert_eq!(location.path, "/?x=1");
    }

    #[test]
    fn refuses_missing_scheme() {
        assert!(matches!(
            Location::parse("example.test/path"),
            Err(ErrorCode::HttpRequestUriInvalid)
        ));
    }

    #[test]
    fn refuses_empty_authority() {
        assert!(matches!(
            Location::parse("https:///path"),
            Err(ErrorCode::HttpRequestUriInvalid)
        ));
    }

    #[test]
    fn parses_normal_url() {
        let location = Location::parse("https://example.test/path?q=yes").unwrap();
        assert!(matches!(location.scheme, Scheme::Https));
        assert_eq!(location.authority, "example.test");
        assert_eq!(location.path, "/path?q=yes");
    }
}

export!(Component);
