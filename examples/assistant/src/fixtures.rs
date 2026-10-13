//! Loopback services local to this example.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
#[cfg(test)]
use std::thread::JoinHandle;
use std::time::Duration;

const RESEARCH: &str = "WebAssembly components are portable units with explicit interfaces.";
const WEATHER: &str = "Sunny, 21°C — an entirely made-up forecast.";
const COLLECTED: &str = "collected";

/// A three-request server for the example's page and tool endpoints.
pub struct Fixtures {
    address: SocketAddr,
    weather_authority: String,
    collector_authority: String,
    #[cfg(test)]
    thread: JoinHandle<std::io::Result<()>>,
}

impl Fixtures {
    /// Starts the fixture server on an ephemeral loopback port.
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let thread = thread::spawn(move || {
            serve_one(&listener)?;
            serve_one(&listener)?;
            serve_one(&listener)
        });
        #[cfg(not(test))]
        drop(thread);
        Ok(Self {
            address,
            weather_authority: address.to_string(),
            collector_authority: format!("localhost:{}", address.port()),
            #[cfg(test)]
            thread,
        })
    }

    /// Returns an HTTP URL for one fixture path.
    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    pub(crate) fn weather_authority(&self) -> &str {
        &self.weather_authority
    }

    pub(crate) fn collector_authority(&self) -> &str {
        &self.collector_authority
    }

    /// Waits until all fixture requests have been served.
    #[cfg(test)]
    pub fn finish(self) -> std::io::Result<()> {
        self.thread
            .join()
            .map_err(|_| std::io::Error::other("fixture thread panicked"))?
    }
}

fn serve_one(listener: &TcpListener) -> std::io::Result<()> {
    let (mut stream, _) = listener.accept()?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let request = read_request(&mut stream)?;
    let request = String::from_utf8_lossy(&request);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default();
    let (status, body) = match path.split('?').next() {
        Some("/research") => ("200 OK", RESEARCH),
        Some("/weather") => ("200 OK", WEATHER),
        Some("/collect") => ("200 OK", COLLECTED),
        _ => ("404 Not Found", "not found"),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    let header_end = loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        request.extend_from_slice(&buffer[..count]);
        if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let length = content_length(&request[..header_end]);
    while request.len() < header_end + length {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        request.extend_from_slice(&buffer[..count]);
    }
    Ok(request)
}

fn content_length(headers: &[u8]) -> usize {
    String::from_utf8_lossy(headers)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{COLLECTED, Fixtures, RESEARCH, WEATHER};
    use std::io::{Read, Write};
    use std::net::{Shutdown, SocketAddr, TcpStream};

    #[test]
    fn serves_page_weather_and_collector() {
        let fixtures = Fixtures::start().unwrap();
        assert_eq!(get(fixtures.address, "/research"), RESEARCH);
        assert_eq!(get(fixtures.address, "/weather?city=Boston"), WEATHER);
        assert_eq!(get(fixtures.address, "/collect"), COLLECTED);
        fixtures.finish().unwrap();
    }

    fn get(address: SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(address).unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nContent-Length: 4\r\n\r\ndone"
        )
        .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response.split_once("\r\n\r\n").unwrap().1.into()
    }
}
