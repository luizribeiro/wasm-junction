//! Loopback HTTP server local to this example.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// One-request loopback server used by the example host.
pub struct Server {
    address: SocketAddr,
    request: Receiver<std::io::Result<Vec<u8>>>,
    thread: JoinHandle<()>,
}

impl Server {
    /// Starts the server on an ephemeral loopback port.
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let (sender, request) = mpsc::channel();
        let thread = thread::spawn(move || {
            let result = serve(&listener);
            sender.send(result).ok();
        });
        Ok(Self {
            address,
            request,
            thread,
        })
    }

    /// Returns the authority accepted by the origin policy.
    pub fn authority(&self) -> String {
        self.address.to_string()
    }

    /// Waits for the request and returns its bytes.
    pub fn finish(self) -> std::io::Result<Vec<u8>> {
        let request = self
            .request
            .recv_timeout(Duration::from_secs(5))
            .map_err(std::io::Error::other)??;
        self.thread
            .join()
            .map_err(|_| std::io::Error::other("HTTP server thread panicked"))?;
        Ok(request)
    }
}

fn serve(listener: &TcpListener) -> std::io::Result<Vec<u8>> {
    let (mut stream, _) = listener.accept()?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request
        .windows(b"example-body".len())
        .any(|part| part == b"example-body")
    {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let body = b"hello from server";
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(request)
}
