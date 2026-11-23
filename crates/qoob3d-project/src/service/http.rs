//! Minimal HTTP/1.1 for the service on 127.0.0.1 (AG-SV-01): one request per
//! connection, `Content-Length` bodies, `Connection: close`. Enough for the
//! app's client (`live.rs`), the CLI and agents; no keep-alive, no chunking.
//! The headers that decide whether a request is let in (AG-HD-03, ADR-0055)
//! are read here, the decision is the service's.

use std::fs::File;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;

/// Largest accepted request body; requests are small JSON objects.
const MAX_BODY: usize = 1 << 20;
const MAX_HEAD_LINES: usize = 100;

pub(crate) struct Request {
    pub(crate) method: String,
    /// Path segments, percent-decoded.
    pub(crate) path: Vec<String>,
    pub(crate) body: Vec<u8>,
    /// `X-Qoob3d-Token`.
    pub(crate) token: Option<String>,
    pub(crate) host: Option<String>,
    /// The request carries an `Origin` header: it comes from a web page.
    pub(crate) origin: bool,
}

/// What a handler answers.
pub(crate) enum Reply {
    Json {
        code: u16,
        body: String,
    },
    /// A qmesh file streamed from disk (WA-PR-02).
    Mesh {
        key: String,
        file: File,
    },
}

pub(crate) fn read_request(stream: &TcpStream) -> std::io::Result<Request> {
    let invalid = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_owned());
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut words = line.split_whitespace();
    let (Some(method), Some(target)) = (words.next(), words.next()) else {
        return Err(invalid("malformed request line"));
    };
    let method = method.to_owned();
    let path = target.split('?').next().unwrap_or_default();
    let path = path.split('/').filter(|s| !s.is_empty()).map(percent_decode).collect();
    let (mut length, mut token, mut host, mut origin) = (0, None, None, false);
    for _ in 0..MAX_HEAD_LINES {
        line.clear();
        reader.read_line(&mut line)?;
        let header = line.trim_end();
        if header.is_empty() {
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            return Ok(Request { method, path, body, token, host, origin });
        }
        let Some((name, value)) = header.split_once(':') else { continue };
        let (name, value) = (name.trim(), value.trim());
        if name.eq_ignore_ascii_case("Content-Length") {
            length = value.parse().map_err(|_| invalid("bad Content-Length"))?;
            if length > MAX_BODY {
                return Err(invalid("request body too large"));
            }
        } else if name.eq_ignore_ascii_case("X-Qoob3d-Token") {
            token = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("Host") {
            host = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("Origin") {
            origin = true;
        }
    }
    Err(invalid("too many header lines"))
}

/// `%XX` escapes of a path segment; invalid escapes stay as they are.
fn percent_decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Internal Server Error",
    }
}

pub(crate) fn write_reply(stream: &mut TcpStream, reply: Reply) -> std::io::Result<()> {
    match reply {
        Reply::Json { code, body } => {
            let head = format!(
                "HTTP/1.1 {code} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                reason(code),
                body.len()
            );
            stream.write_all(head.as_bytes())?;
            stream.write_all(body.as_bytes())?;
        }
        Reply::Mesh { key, file } => {
            let length = file.metadata()?.len();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: \
                 {length}\r\nX-Qoob3d-Key: {key}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(head.as_bytes())?;
            std::io::copy(&mut file.take(length), stream)?;
        }
    }
    stream.flush()
}
