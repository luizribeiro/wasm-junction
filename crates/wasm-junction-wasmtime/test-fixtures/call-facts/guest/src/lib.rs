wit_bindgen::generate!({
    path: "../wit",
    world: "fixture",
    generate_all,
});

struct Component;

impl exports::test::call_facts::agent::Guest for Component {
    fn run(path: String, authority: String) -> String {
        let note = test::wasi::environment::read_file(&path)
            .unwrap_or_else(|error| format!("read error: {error}"));
        format!("{note}|{}", send(authority))
    }
}

fn send(authority: String) -> String {
    use wasi::http::outgoing_handler;
    use wasi::http::types::{Fields, Method, OutgoingRequest, Scheme};

    let request = OutgoingRequest::new(Fields::new());
    let _ = request.set_method(&Method::Get);
    let _ = request.set_scheme(Some(&Scheme::Http));
    let _ = request.set_authority(Some(&authority));
    let _ = request.set_path_with_query(Some("/research"));
    match outgoing_handler::handle(request, None) {
        Ok(future) => {
            future.subscribe().block();
            match future.get() {
                Some(Ok(Ok(response))) => {
                    drop(response);
                    "sent".into()
                }
                Some(Ok(Err(wasi::http::types::ErrorCode::HttpRequestDenied))) => "denied".into(),
                Some(Ok(Err(error))) => format!("error: {error:?}"),
                Some(Err(())) => "error: response consumed".into(),
                None => "error: response not ready".into(),
            }
        }
        Err(wasi::http::types::ErrorCode::HttpRequestDenied) => "denied".into(),
        Err(error) => format!("error: {error:?}"),
    }
}

export!(Component);
