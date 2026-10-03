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
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request
                    .windows(b"guest-body".len())
                    .any(|part| part == b"guest-body")
                {
                    break;
                }
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
