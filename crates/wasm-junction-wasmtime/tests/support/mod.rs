#![cfg(feature = "wasi-http")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct HttpServer {
    address: SocketAddr,
    request: Receiver<Vec<u8>>,
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

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|part| part == needle)
}

impl HttpServer {
    pub fn start(response_body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, request) = mpsc::channel();
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            // Answering before the whole request arrives closes the socket with unread data,
            // which resets the connection while the client is still writing its body.
            while !request_complete(&request) {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
            }
            sender.send(request).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            )
            .unwrap();
            stream.write_all(response_body).unwrap();
        });
        Self {
            address,
            request,
            thread,
        }
    }

    pub fn authority(&self) -> String {
        self.address.to_string()
    }

    pub fn finish(self) -> String {
        let request = self.request.recv_timeout(Duration::from_secs(5)).unwrap();
        self.thread.join().unwrap();
        String::from_utf8_lossy(&request).into_owned()
    }
}
