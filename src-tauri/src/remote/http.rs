//! A small blocking HTTP/1.1 client for the `cr worker` API: JSON requests and Server-Sent Events,
//! over the local end of a tunnel (a unix socket or a loopback port). One connection per request
//! (`Connection: close`), which keeps it free of pooling and half-open-connection problems.
//!
//! No HTTP crate is used: the app has no blocking client (reqwest is built without it) and the API
//! is tiny. Anything that is not a plain, well-formed answer is an error, never a guess.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use super::error::{RemoteError, RemoteResult};
use super::tunnel::Addr;

trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

type Conn = Box<dyn Stream>;

/// A `Read` over a stream that can set its read timeout.
struct Connection {
    inner: Conn,
}

fn connect(addr: &Addr, connect_timeout: Duration, read_timeout: Duration) -> RemoteResult<Connection> {
    let unreachable = |e: std::io::Error| RemoteError::unreachable(format!("cannot connect to the tunnel: {e}"));
    let conn: Conn = match addr {
        #[cfg(unix)]
        Addr::Unix(path) => {
            let s = std::os::unix::net::UnixStream::connect(path).map_err(unreachable)?;
            s.set_read_timeout(Some(read_timeout)).map_err(unreachable)?;
            s.set_write_timeout(Some(connect_timeout)).map_err(unreachable)?;
            Box::new(s)
        }
        #[cfg(not(unix))]
        Addr::Unix(_) => return Err(RemoteError::unreachable("unix sockets are not available on this platform")),
        Addr::Tcp(port) => {
            let s = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, *port)), connect_timeout).map_err(unreachable)?;
            s.set_read_timeout(Some(read_timeout)).map_err(unreachable)?;
            s.set_write_timeout(Some(connect_timeout)).map_err(unreachable)?;
            Box::new(s)
        }
    };
    Ok(Connection { inner: conn })
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

fn host_header(addr: &Addr) -> String {
    match addr {
        Addr::Unix(_) => "localhost".into(),
        Addr::Tcp(port) => format!("127.0.0.1:{port}"),
    }
}

fn send(conn: &mut Connection, addr: &Addr, method: &str, path: &str, body: Option<&[u8]>) -> RemoteResult<()> {
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: */*\r\n", host_header(addr));
    if let Some(b) = body {
        head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    }
    head.push_str("\r\n");
    let io = |e: std::io::Error| RemoteError::unreachable(format!("connection lost while sending: {e}"));
    conn.inner.write_all(head.as_bytes()).map_err(io)?;
    if let Some(b) = body {
        conn.inner.write_all(b).map_err(io)?;
    }
    conn.inner.flush().map_err(io)
}

#[derive(Debug)]
enum Framing {
    Length(u64),
    Chunked,
    /// Until the peer closes.
    Eof,
}

#[derive(Debug)]
struct Head {
    status: u16,
    framing: Framing,
}

fn read_head<R: BufRead>(r: &mut R) -> RemoteResult<Head> {
    let lost = |e: std::io::Error| RemoteError::unreachable(format!("connection lost: {e}"));
    let mut line = String::new();
    if r.read_line(&mut line).map_err(lost)? == 0 {
        return Err(RemoteError::unreachable("the connection closed before an answer"));
    }
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| RemoteError::protocol(format!("not an HTTP answer: {}", line.trim())))?;
    let mut framing = Framing::Eof;
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(lost)? == 0 {
            return Err(RemoteError::unreachable("the connection closed in the middle of the headers"));
        }
        let h = line.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((name, value)) = h.split_once(':') {
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
            if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
                framing = Framing::Chunked;
            } else if name == "content-length" && !matches!(framing, Framing::Chunked) {
                framing = Framing::Length(value.parse().map_err(|_| RemoteError::protocol("bad content-length"))?);
            }
        }
    }
    Ok(Head { status, framing })
}

/// The body of an answer, de-framed.
struct Body<R: BufRead> {
    r: R,
    framing: Framing,
    /// Bytes left in the current chunk / the length-framed body.
    left: u64,
    done: bool,
}

impl<R: BufRead> Body<R> {
    fn new(r: R, framing: Framing) -> Self {
        let left = if let Framing::Length(n) = framing { n } else { 0 };
        Self { r, framing, left, done: false }
    }

    fn next_chunk(&mut self) -> std::io::Result<()> {
        let mut line = String::new();
        self.r.read_line(&mut line)?;
        let size = line.trim().split(';').next().unwrap_or("");
        let n = u64::from_str_radix(size, 16).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size"))?;
        if n == 0 {
            // trailers, then the final blank line
            loop {
                line.clear();
                if self.r.read_line(&mut line)? == 0 || line.trim().is_empty() {
                    break;
                }
            }
            self.done = true;
        }
        self.left = n;
        Ok(())
    }
}

impl<R: BufRead> Read for Body<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        match self.framing {
            Framing::Eof => self.r.read(buf),
            Framing::Length(_) => {
                if self.left == 0 {
                    self.done = true;
                    return Ok(0);
                }
                let max = buf.len().min(self.left as usize);
                let n = self.r.read(&mut buf[..max])?;
                if n == 0 {
                    return Err(std::io::ErrorKind::UnexpectedEof.into());
                }
                self.left -= n as u64;
                Ok(n)
            }
            Framing::Chunked => {
                if self.left == 0 {
                    self.next_chunk()?;
                    if self.done {
                        return Ok(0);
                    }
                }
                let max = buf.len().min(self.left as usize);
                let n = self.r.read(&mut buf[..max])?;
                if n == 0 {
                    return Err(std::io::ErrorKind::UnexpectedEof.into());
                }
                self.left -= n as u64;
                if self.left == 0 {
                    let mut crlf = [0u8; 2];
                    self.r.read_exact(&mut crlf)?;
                }
                Ok(n)
            }
        }
    }
}

/// An answer with its whole body.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// The largest answer body `request` accepts (a run's text is the biggest thing a daemon sends).
pub const MAX_BODY: usize = 64 * 1024 * 1024;

/// Bounds on one request beyond the per-read timeout.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A longer body is a protocol error, not memory.
    pub max_body: usize,
    /// The whole body must arrive within this; a host dripping bytes fails instead of holding on.
    pub total: Option<Duration>,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_body: MAX_BODY, total: None }
    }
}

/// One request, the whole answer read. `timeout` bounds connecting, sending and every read.
pub fn request(addr: &Addr, method: &str, path: &str, body: Option<&[u8]>, timeout: Duration) -> RemoteResult<Response> {
    request_limited(addr, method, path, body, timeout, Limits::default())
}

/// [`request`] with a cap on the answer's size and on the total time spent reading it.
pub fn request_limited(addr: &Addr, method: &str, path: &str, body: Option<&[u8]>, timeout: Duration, limits: Limits) -> RemoteResult<Response> {
    let started = std::time::Instant::now();
    let mut conn = connect(addr, timeout.min(Duration::from_secs(10)), timeout)?;
    send(&mut conn, addr, method, path, body)?;
    let mut reader = BufReader::new(conn);
    let head = read_head(&mut reader)?;
    let mut body = Body::new(reader, head.framing);
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        if limits.total.is_some_and(|t| started.elapsed() > t) {
            return Err(RemoteError::unreachable("timed out reading the answer"));
        }
        let n = body.read(&mut buf).map_err(|e| RemoteError::unreachable(format!("connection lost while reading the answer: {e}")))?;
        if n == 0 {
            break;
        }
        if out.len() + n > limits.max_body {
            return Err(RemoteError::protocol(format!("the answer is larger than {} bytes", limits.max_body)));
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(Response { status: head.status, body: out })
}

/// One Server-Sent Event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub id: Option<String>,
    pub data: String,
}

/// An open event stream. `next` blocks up to the read timeout per event (the daemon sends a
/// keep-alive comment every 15 s, so a longer silence means the connection is dead).
pub struct SseReader {
    lines: BufReader<Body<BufReader<Connection>>>,
}

/// `GET path` as an event stream. Non-2xx answers are returned as `Err(Response)` bodies by the caller.
pub fn open_sse(addr: &Addr, path: &str, read_timeout: Duration) -> RemoteResult<Result<SseReader, Response>> {
    let mut conn = connect(addr, Duration::from_secs(10), read_timeout)?;
    send(&mut conn, addr, "GET", path, None)?;
    let mut reader = BufReader::new(conn);
    let head = read_head(&mut reader)?;
    let mut body = Body::new(reader, head.framing);
    if !(200..300).contains(&head.status) {
        let mut out = Vec::new();
        let _ = body.read_to_end(&mut out);
        return Ok(Err(Response { status: head.status, body: out }));
    }
    Ok(Ok(SseReader { lines: BufReader::new(body) }))
}

impl SseReader {
    /// The next event, or `None` when the server closed the stream cleanly.
    pub fn next_event(&mut self) -> RemoteResult<Option<SseEvent>> {
        let (mut event, mut id, mut data) = (String::new(), None, Vec::<String>::new());
        let mut any = false;
        loop {
            let mut line = String::new();
            let n = self.lines.read_line(&mut line).map_err(|e| match e.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                    RemoteError::unreachable("the event stream went silent (no data or keep-alive)")
                }
                _ => RemoteError::unreachable(format!("the event stream was cut: {e}")),
            })?;
            if n == 0 {
                // A clean close between events ends the stream; a half event is a cut.
                return if any { Err(RemoteError::unreachable("the event stream was cut in the middle of an event")) } else { Ok(None) };
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if any {
                    return Ok(Some(SseEvent { event: if event.is_empty() { "message".into() } else { event }, id, data: data.join("\n") }));
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            any = true;
            let (field, value) = line.split_once(':').map_or((line, ""), |(f, v)| (f, v.strip_prefix(' ').unwrap_or(v)));
            match field {
                "event" => event = value.to_string(),
                "id" => id = Some(value.to_string()),
                "data" => data.push(value.to_string()),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn body_of(raw: &str, framing: Framing) -> String {
        let mut out = String::new();
        Body::new(Cursor::new(raw.as_bytes().to_vec()), framing).read_to_string(&mut out).unwrap();
        out
    }

    /// A one-connection server on a loopback port: sends `head`, then `body` chunks with `pause` between.
    fn serve_once(head: &'static str, chunks: Vec<Vec<u8>>, pause: Duration) -> Addr {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::Write;
            let Ok((mut c, _)) = listener.accept() else { return };
            let mut buf = [0u8; 1024];
            let _ = c.read(&mut buf);
            let _ = c.write_all(head.as_bytes());
            for chunk in chunks {
                if c.write_all(&chunk).is_err() {
                    return;
                }
                let _ = c.flush();
                std::thread::sleep(pause);
            }
        });
        Addr::Tcp(port)
    }

    #[test]
    fn an_oversized_body_is_a_protocol_error_not_memory() {
        let addr = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\n", vec![vec![b'x'; 100_000]], Duration::ZERO);
        let limits = Limits { max_body: 4096, total: None };
        let err = request_limited(&addr, "GET", "/v1/sessions", None, Duration::from_secs(5), limits).unwrap_err();
        assert_eq!(err.code, "bad_response", "{err}");
        // within the cap it is read whole
        let addr = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n", vec![b"hello".to_vec()], Duration::ZERO);
        let ok = request_limited(&addr, "GET", "/", None, Duration::from_secs(5), limits).unwrap();
        assert_eq!(ok.body, b"hello");
    }

    #[test]
    fn a_slow_drip_host_hits_the_total_deadline() {
        let chunks = (0..40).map(|_| vec![b'x']).collect();
        let addr = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 40\r\n\r\n", chunks, Duration::from_millis(50));
        let limits = Limits { max_body: 1024, total: Some(Duration::from_millis(300)) };
        let started = std::time::Instant::now();
        let err = request_limited(&addr, "GET", "/", None, Duration::from_secs(5), limits).unwrap_err();
        assert!(err.is_connection() && err.message.contains("timed out"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn bodies_are_deframed() {
        assert_eq!(body_of("hello", Framing::Length(5)), "hello");
        assert_eq!(body_of("5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n", Framing::Chunked), "hello world");
        assert_eq!(body_of("until close", Framing::Eof), "until close");
        let mut out = String::new();
        let err = Body::new(Cursor::new(b"abc".to_vec()), Framing::Length(10)).read_to_string(&mut out).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn heads_give_status_and_framing() {
        let mut r = Cursor::new(b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\ncontent-length: 7\r\n\r\n".to_vec());
        let h = read_head(&mut r).unwrap();
        assert!(h.status == 201 && matches!(h.framing, Framing::Length(7)));
        let mut r = Cursor::new(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec());
        assert!(matches!(read_head(&mut r).unwrap().framing, Framing::Chunked));
        assert!(read_head(&mut Cursor::new(b"garbage\r\n\r\n".to_vec())).is_err());
        assert!(read_head(&mut Cursor::new(Vec::new())).unwrap_err().is_connection());
    }

    fn sse_of(raw: &str) -> SseReader {
        SseReader { lines: BufReader::new(Body::new(BufReader::new(Connection { inner: Box::new(Cursor::new(raw.as_bytes().to_vec())) }), Framing::Eof)) }
    }

    #[test]
    fn events_are_parsed_and_keepalives_skipped() {
        let mut r = sse_of(": keep-alive\n\nevent: event\nid: 3\ndata: {\"seq\":3}\n\nevent: status\ndata: {\"status\":\"ok\"}\n\n");
        assert_eq!(r.next_event().unwrap().unwrap(), SseEvent { event: "event".into(), id: Some("3".into()), data: "{\"seq\":3}".into() });
        let s = r.next_event().unwrap().unwrap();
        assert_eq!((s.event.as_str(), s.id), ("status", None));
        assert_eq!(r.next_event().unwrap(), None);
    }

    #[test]
    fn multi_line_data_is_joined_and_a_cut_event_is_an_error() {
        let mut r = sse_of("data: a\ndata: b\n\n");
        assert_eq!(r.next_event().unwrap().unwrap().data, "a\nb");
        let mut cut = sse_of("event: event\ndata: {\"se");
        assert!(cut.next_event().unwrap_err().is_connection());
    }
}
