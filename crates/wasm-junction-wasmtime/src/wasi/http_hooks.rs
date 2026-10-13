use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use http::uri::Scheme;
use http_body::Body;
use http_body_util::BodyExt;
use tokio::sync::oneshot;
use wasmtime_wasi::runtime::{AbortOnDropJoinHandle, spawn};
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks, default_send_request};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) struct HttpHooks;

impl WasiHttpHooks for HttpHooks {
    fn send_request(
        &mut self,
        request: http::Request<WasiBody>,
        options: Option<RequestOptions>,
        _body_result: Box<dyn Future<Output = Result<(), Error>> + Send>,
    ) -> Box<
        dyn Future<
                Output = Result<
                    (
                        http::Response<WasiBody>,
                        Box<dyn Future<Output = Result<(), Error>> + Send>,
                    ),
                    Error,
                >,
            > + Send,
    > {
        let deadline = https_response_deadline(&request, options);
        Box::new(async move {
            let (request, request_result) = RequestBody::wrap(request);
            let send = default_send_request(request, options);
            let (response, io) = if let Some(deadline) = deadline {
                tokio::time::timeout(deadline, send)
                    .await
                    .map_err(|_| Error::ConnectionTimeout)??
            } else {
                send.await?
            };
            let (connection, connection_result) = ConnectionTask::spawn(io);
            let response = response.map(|body| {
                ConnectionBody::new(body.boxed_unsync(), connection.clone(), connection_result)
                    .boxed_unsync()
            });
            Ok((
                response,
                Box::new(async move {
                    let _connection = connection;
                    match request_result.await {
                        Ok(true) => Ok(()),
                        Ok(false) | Err(_) => Err(Error::ConnectionTerminated),
                    }
                }) as Box<dyn Future<Output = _> + Send>,
            ))
        })
    }
}

struct RequestBody {
    body: WasiBody,
    result: Option<oneshot::Sender<bool>>,
}

impl RequestBody {
    fn wrap(request: http::Request<WasiBody>) -> (http::Request<Self>, oneshot::Receiver<bool>) {
        // Hyper does not poll unknown-length bodies for these methods unless framing opts in.
        let implicit_empty = matches!(
            *request.method(),
            http::Method::GET | http::Method::HEAD | http::Method::CONNECT
        ) && !request.headers().contains_key(http::header::CONTENT_LENGTH)
            && !request
                .headers()
                .contains_key(http::header::TRANSFER_ENCODING);
        let (parts, body) = request.into_parts();
        let (result, receiver) = oneshot::channel();
        let mut body = Self {
            body,
            result: Some(result),
        };
        if implicit_empty || body.body.is_end_stream() || body.body.size_hint().exact() == Some(0) {
            body.finish(true);
        }
        (http::Request::from_parts(parts, body), receiver)
    }

    fn finish(&mut self, sent: bool) {
        if let Some(result) = self.result.take() {
            _ = result.send(sent);
        }
    }
}

impl Body for RequestBody {
    type Data = <WasiBody as Body>::Data;
    type Error = Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let frame = Pin::new(&mut self.body).poll_frame(context);
        if matches!(frame, Poll::Ready(None))
            || matches!(frame, Poll::Ready(Some(Ok(_)))) && self.body.is_end_stream()
        {
            self.finish(true);
        } else if matches!(frame, Poll::Ready(Some(Err(_)))) {
            self.finish(false);
        }
        frame
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}

impl Drop for RequestBody {
    fn drop(&mut self) {
        self.finish(false);
    }
}

struct ConnectionTask {
    _handle: AbortOnDropJoinHandle<()>,
}

impl ConnectionTask {
    fn spawn(
        io: impl Future<Output = Result<(), Error>> + Send + 'static,
    ) -> (Arc<Self>, oneshot::Receiver<Result<(), Error>>) {
        let (result, receiver) = oneshot::channel();
        let handle = spawn(async move {
            _ = result.send(io.await);
        });
        (Arc::new(Self { _handle: handle }), receiver)
    }
}

struct ConnectionBody {
    body: WasiBody,
    _connection: Arc<ConnectionTask>,
    result: Option<oneshot::Receiver<Result<(), Error>>>,
}

impl ConnectionBody {
    fn new(
        body: WasiBody,
        connection: Arc<ConnectionTask>,
        result: oneshot::Receiver<Result<(), Error>>,
    ) -> Self {
        Self {
            body,
            _connection: connection,
            result: Some(result),
        }
    }
}

impl Body for ConnectionBody {
    type Data = <WasiBody as Body>::Data;
    type Error = Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        if let Some(receiver) = self.result.as_mut()
            && let Poll::Ready(result) = Pin::new(receiver).poll(context)
        {
            self.result = None;
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Poll::Ready(Some(Err(error))),
                Err(_) => return Poll::Ready(Some(Err(Error::ConnectionTerminated))),
            }
        }
        Pin::new(&mut self.body).poll_frame(context)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}

fn https_response_deadline<B>(
    request: &http::Request<B>,
    options: Option<RequestOptions>,
) -> Option<Duration> {
    (request.uri().scheme() == Some(&Scheme::HTTPS)).then(|| {
        // The upstream sender applies these phases separately but leaves the TLS handshake
        // unbounded, so their sum is the least restrictive deadline that still covers it.
        let connect = options
            .and_then(|options| options.connect_timeout)
            .unwrap_or(DEFAULT_TIMEOUT);
        let first_byte = options
            .and_then(|options| options.first_byte_timeout)
            .unwrap_or(DEFAULT_TIMEOUT);
        connect.saturating_add(first_byte)
    })
}

#[cfg(test)]
mod tests {
    use super::{ConnectionBody, ConnectionTask, DEFAULT_TIMEOUT, https_response_deadline};
    use http_body_util::{BodyExt, Empty};
    use std::convert::Infallible;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use wasmtime_wasi_http::{Error, RequestOptions, WasiBody};

    fn empty_body() -> WasiBody {
        Empty::new()
            .map_err(|error: Infallible| match error {})
            .boxed_unsync()
    }

    struct PendingBody;

    impl http_body::Body for PendingBody {
        type Data = <WasiBody as http_body::Body>::Data;
        type Error = Error;

        fn poll_frame(
            self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            std::task::Poll::Pending
        }
    }

    #[test]
    fn connection_driver_waits_for_all_owners() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_sender, accepted) = mpsc::channel();
        let (closed_sender, closed) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            accepted_sender.send(()).unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
            closed_sender.send(()).unwrap();
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async {
            let (connection, result) = ConnectionTask::spawn(async move {
                let stream = tokio::net::TcpStream::connect(address)
                    .await
                    .map_err(Error::Connect)?;
                std::future::pending::<()>().await;
                drop(stream);
                Ok(())
            });
            let completion = connection.clone();
            let body = ConnectionBody::new(empty_body(), connection, result);
            while accepted.try_recv().is_err() {
                tokio::task::yield_now().await;
            }
            drop(body);
            assert!(matches!(
                closed.recv_timeout(Duration::from_millis(500)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            drop(completion);
            tokio::task::yield_now().await;
        });

        closed.recv_timeout(Duration::from_secs(5)).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn connection_driver_error_reaches_the_body() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (connection, result) = ConnectionTask::spawn(async { Err(Error::ConnectionTimeout) });
        let mut body = ConnectionBody::new(PendingBody.boxed_unsync(), connection, result);

        let error = runtime.block_on(async { body.frame().await.unwrap().unwrap_err() });

        assert!(matches!(error, Error::ConnectionTimeout));
    }

    #[test]
    fn response_deadline_uses_https_phase_budget() {
        let configured = RequestOptions {
            connect_timeout: Some(Duration::from_secs(2)),
            first_byte_timeout: Some(Duration::from_secs(3)),
            between_bytes_timeout: Some(Duration::from_secs(7)),
        };
        let connect_only = RequestOptions {
            connect_timeout: Some(Duration::from_secs(2)),
            ..RequestOptions::default()
        };
        let cases = [
            ("http://example.test/", None, None),
            (
                "https://example.test/",
                None,
                Some(DEFAULT_TIMEOUT.saturating_add(DEFAULT_TIMEOUT)),
            ),
            (
                "https://example.test/",
                Some(configured),
                Some(Duration::from_secs(5)),
            ),
            (
                "https://example.test/",
                Some(connect_only),
                Some(Duration::from_secs(2).saturating_add(DEFAULT_TIMEOUT)),
            ),
        ];

        for (uri, options, expected) in cases {
            let request = http::Request::builder().uri(uri).body(()).unwrap();
            assert_eq!(https_response_deadline(&request, options), expected);
        }
    }
}
