#![cfg(feature = "wasi-http")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[allow(
    dead_code,
    reason = "each integration-test binary uses a different subset of server probes"
)]
pub struct HttpServer {
    address: SocketAddr,
    request: Receiver<Vec<u8>>,
    headers_sent: Arc<AtomicBool>,
    connection_closed: Arc<AtomicBool>,
    stop: Option<Sender<()>>,
    thread: JoinHandle<()>,
}

fn request_complete(request: &[u8]) -> bool {
    let Some(end) = find_subslice(request, b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
    let body = &request[end + 4..];
    if head.contains("transfer-encoding: chunked") {
        return find_subslice(body, b"0\r\n\r\n").is_some();
    }
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    body.len() >= length
}

fn request_headers_complete(request: &[u8]) -> bool {
    find_subslice(request, b"\r\n\r\n").is_some()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|part| part == needle)
}

#[allow(
    dead_code,
    reason = "each integration-test binary uses a different subset of server probes"
)]
impl HttpServer {
    pub fn start(response_body: &'static [u8]) -> Self {
        Self::start_with_connection(response_body, false)
    }

    pub fn respond_before_request_body() -> Self {
        Self::start_with_response(b"", true, true, true)
    }

    #[allow(
        dead_code,
        reason = "only HTTP integration-test binaries exercise a stalled response body"
    )]
    pub fn stall_after_headers() -> Self {
        Self::start_with_response(b"", true, false, false)
    }

    fn start_with_connection(response_body: &'static [u8], keep_alive: bool) -> Self {
        Self::start_with_response(response_body, keep_alive, true, false)
    }

    fn start_with_response(
        response_body: &'static [u8],
        keep_alive: bool,
        complete_body: bool,
        respond_before_body: bool,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, request) = mpsc::channel();
        let headers_sent = Arc::new(AtomicBool::new(false));
        let thread_headers_sent = headers_sent.clone();
        let connection_closed = Arc::new(AtomicBool::new(false));
        let thread_connection_closed = connection_closed.clone();
        let (stop, stopped) = mpsc::channel();
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            // Some probes answer after headers to exercise response and upload lifetimes together.
            while !(if respond_before_body {
                request_headers_complete(&request)
            } else {
                request_complete(&request)
            }) {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
            }
            let connection = if keep_alive { "keep-alive" } else { "close" };
            let content_length = if complete_body {
                response_body.len()
            } else {
                response_body.len() + 1
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {content_length}\r\nConnection: {connection}\r\n\r\n",
            )
            .unwrap();
            thread_headers_sent.store(true, Ordering::Release);
            if complete_body {
                stream.write_all(response_body).unwrap();
            }
            if respond_before_body {
                while !request_complete(&request) {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
            }
            sender.send(request).unwrap();
            if keep_alive {
                stream
                    .set_read_timeout(Some(Duration::from_millis(50)))
                    .unwrap();
                loop {
                    if stopped.try_recv().is_ok() {
                        break;
                    }
                    match stream.read(&mut buffer) {
                        Ok(0) => {
                            thread_connection_closed.store(true, Ordering::Release);
                            break;
                        }
                        Ok(_) => {}
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(_) => {
                            thread_connection_closed.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            }
        });
        Self {
            address,
            request,
            headers_sent,
            connection_closed,
            stop: keep_alive.then_some(stop),
            thread,
        }
    }

    pub fn authority(&self) -> String {
        self.address.to_string()
    }

    pub fn has_sent_headers(&self) -> bool {
        self.headers_sent.load(Ordering::Acquire)
    }

    pub fn connection_closed(&self) -> bool {
        self.connection_closed.load(Ordering::Acquire)
    }

    pub fn finish(mut self) -> String {
        let request = self.request.recv_timeout(Duration::from_secs(5)).unwrap();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.thread.join().unwrap();
        String::from_utf8_lossy(&request).into_owned()
    }
}
