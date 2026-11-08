//! A minimal HTTP/1.1 client for the pinned downloads: `GET` only, one request per
//! connection, TLS with rustls (the pure-Rust cryptography provider of `super::tls`) and
//! the Mozilla root certificates of webpki-roots (no system library). Redirects are followed a few times, never from HTTPS to plain HTTP;
//! plain HTTP is only allowed to a loopback address (a local mirror or test server).
//! The response is read with a size limit, and a download interrupted half-way resumes
//! where it stopped (`Range`) when the server allows it. What is downloaded is checked
//! against its pinned digest by the caller: TLS is a second line of defence, not the
//! first.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// The `User-Agent` of every request.
pub(crate) fn user_agent() -> String {
    format!(
        "einvoice-validator/{} (+https://github.com/einvoice-validator/einvoice-validator)",
        crate::VERSION
    )
}

/// The most redirects followed for one request.
pub(crate) const MAX_REDIRECTS: u32 = 5;
/// The most bytes of a response head (status line and headers).
const MAX_HEAD: usize = 64 << 10;
/// The most header lines of a response.
const MAX_HEADERS: usize = 128;

/// Why a request failed, and whether trying again may help.
#[derive(Debug)]
pub(crate) struct HttpError {
    pub(crate) message: String,
    pub(crate) retryable: bool,
    /// The delay the server asked for (`Retry-After`, in seconds), if any.
    pub(crate) retry_after: Option<u64>,
}

impl HttpError {
    fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
            retry_after: None,
        }
    }

    fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
            retry_after: None,
        }
    }
}

/// A URL the client can fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Url {
    pub(crate) tls: bool,
    pub(crate) host: String,
    pub(crate) port: u16,
    /// The path and query, starting with `/`.
    pub(crate) target: String,
}

impl Url {
    /// Parses an absolute `https://` URL, or an `http://` one to a loopback host.
    pub(crate) fn parse(url: &str) -> Result<Self, String> {
        let (tls, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(format!("{url}: not an https:// URL"));
        };
        let (authority, target) = match rest.find(['/', '?']) {
            Some(i) if rest.as_bytes().get(i) == Some(&b'/') => (&rest[..i], rest[i..].to_owned()),
            Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
            None => (rest, "/".to_owned()),
        };
        let target = target.split('#').next().unwrap_or("/").to_owned();
        if authority.contains('@') {
            return Err(format!("{url}: credentials in a URL are not supported"));
        }
        let (host, port) = if let Some(after) = authority.strip_prefix('[') {
            let (host, rest) = after
                .split_once(']')
                .ok_or_else(|| format!("{url}: bad IPv6 address"))?;
            (host.to_owned(), rest.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') {
                Some((host, port)) => (host.to_owned(), Some(port)),
                None => (authority.to_owned(), None),
            }
        };
        let port = match port {
            None | Some("") => {
                if tls {
                    443
                } else {
                    80
                }
            }
            Some(p) => p
                .parse::<u16>()
                .map_err(|_| format!("{url}: bad port {p:?}"))?,
        };
        if host.is_empty()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'_'))
        {
            return Err(format!("{url}: bad host {host:?}"));
        }
        if target.bytes().any(|b| b <= b' ' || b == 0x7f) {
            return Err(format!("{url}: spaces or control characters in the path"));
        }
        let url_ok = Self {
            tls,
            host: host.to_ascii_lowercase(),
            port,
            target,
        };
        if !tls && !url_ok.is_loopback() {
            return Err(format!(
                "{url}: plain HTTP is only allowed to a loopback address; use https://"
            ));
        }
        Ok(url_ok)
    }

    fn is_loopback(&self) -> bool {
        self.host == "localhost" || self.host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    }

    /// The URL `location` (a redirect target) names, relative to this one.
    fn join(&self, location: &str) -> Result<Self, String> {
        if location.starts_with("https://") || location.starts_with("http://") {
            let next = Self::parse(location)?;
            if self.tls && !next.tls {
                return Err(format!(
                    "redirect from HTTPS to plain HTTP ({location}) refused"
                ));
            }
            return Ok(next);
        }
        if location.starts_with("//") {
            let scheme = if self.tls { "https:" } else { "http:" };
            return self.join(&format!("{scheme}{location}"));
        }
        let target = if location.starts_with('/') {
            location.to_owned()
        } else {
            let dir = self
                .target
                .split('?')
                .next()
                .and_then(|p| p.rsplit_once('/'))
                .map_or("", |(d, _)| d);
            format!("{dir}/{location}")
        };
        Self::parse(&format!("{}{target}", self.origin()))
    }

    fn origin(&self) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let default = if self.tls { 443 } else { 80 };
        if self.port == default {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}:{}", self.port)
        }
    }

    fn host_header(&self) -> String {
        self.origin()
            .split_once("://")
            .map_or_else(String::new, |(_, h)| h.to_owned())
    }
}

impl std::fmt::Display for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.origin(), self.target)
    }
}

/// The TLS configuration: the pure-Rust provider of [`super::tls`], rustls' safe default
/// protocol versions (TLS 1.2 and 1.3), the Mozilla roots.
fn tls_config() -> Result<Arc<rustls::ClientConfig>, HttpError> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            rustls::ClientConfig::builder_with_provider(Arc::new(super::tls::provider()))
                .with_safe_default_protocol_versions()
                .map(|b| Arc::new(b.with_root_certificates(roots).with_no_client_auth()))
                .map_err(|e| format!("TLS configuration: {e}"))
        })
        .clone()
        .map_err(HttpError::fatal)
}

/// A response: its status, headers and body.
pub(crate) struct Response {
    pub(crate) status: u16,
    headers: Vec<(String, String)>,
    body: Body,
    /// The URL that gave it, after redirects.
    pub(crate) url: Url,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl Response {
    /// The value of the header `name` (case-insensitive), the first if repeated.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The length the body announces (`Content-Length`), when it does.
    pub(crate) fn content_length(&self) -> Option<u64> {
        match self.body.framing {
            Framing::Length(n) => Some(n),
            Framing::Chunked | Framing::Close => None,
        }
    }

    /// Writes the body to `sink`, at most `limit` bytes. Returns the number of bytes
    /// written.
    pub(crate) fn copy_to(&mut self, sink: &mut dyn Write, limit: u64) -> Result<u64, HttpError> {
        let url = self.url.to_string();
        self.body.copy_to(sink, limit, &url)
    }
}

/// How the end of a body is known.
#[derive(Debug, Clone, Copy)]
enum Framing {
    Length(u64),
    Chunked,
    Close,
}

struct Body {
    reader: BufReader<Box<dyn ReadWrite>>,
    framing: Framing,
}

/// A connection: plain TCP or TLS over TCP.
trait ReadWrite: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWrite for T {}

fn io_error(url: &str, e: &std::io::Error) -> HttpError {
    HttpError::transient(format!("{url}: {e}"))
}

impl Body {
    fn copy_to(&mut self, sink: &mut dyn Write, limit: u64, url: &str) -> Result<u64, HttpError> {
        let too_large =
            || HttpError::fatal(format!("{url}: larger than the limit of {limit} bytes"));
        let mut written: u64 = 0;
        let mut buffer = vec![0u8; 64 << 10];
        let mut put = |bytes: &[u8], written: &mut u64| -> Result<(), HttpError> {
            *written = written.saturating_add(bytes.len() as u64);
            if *written > limit {
                return Err(too_large());
            }
            sink.write_all(bytes)
                .map_err(|e| HttpError::fatal(format!("cannot write the download of {url}: {e}")))
        };
        match self.framing {
            Framing::Length(n) => {
                if n > limit {
                    return Err(too_large());
                }
                let mut left = n;
                while left > 0 {
                    let want =
                        usize::try_from(left.min(buffer.len() as u64)).unwrap_or(buffer.len());
                    let got = self
                        .reader
                        .read(&mut buffer[..want])
                        .map_err(|e| io_error(url, &e))?;
                    if got == 0 {
                        return Err(HttpError::transient(format!(
                            "{url}: connection closed after {} of {n} bytes",
                            n - left
                        )));
                    }
                    put(&buffer[..got], &mut written)?;
                    left -= got as u64;
                }
            }
            Framing::Close => loop {
                let got = match self.reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(got) => got,
                    // A TLS peer that closes without close_notify: the end of a body
                    // that has no length (the digest checks it is complete).
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => return Err(io_error(url, &e)),
                };
                put(&buffer[..got], &mut written)?;
            },
            Framing::Chunked => loop {
                let line = read_line(&mut self.reader, url)?;
                let size = line.split(';').next().unwrap_or("").trim();
                let size = u64::from_str_radix(size, 16)
                    .map_err(|_| HttpError::fatal(format!("{url}: bad chunk size {line:?}")))?;
                if size == 0 {
                    // Trailers, up to the empty line.
                    while !read_line(&mut self.reader, url)?.is_empty() {}
                    break;
                }
                if written.saturating_add(size) > limit {
                    return Err(too_large());
                }
                let mut left = size;
                while left > 0 {
                    let want =
                        usize::try_from(left.min(buffer.len() as u64)).unwrap_or(buffer.len());
                    let got = self
                        .reader
                        .read(&mut buffer[..want])
                        .map_err(|e| io_error(url, &e))?;
                    if got == 0 {
                        return Err(HttpError::transient(format!(
                            "{url}: connection closed in a chunk"
                        )));
                    }
                    put(&buffer[..got], &mut written)?;
                    left -= got as u64;
                }
                if !read_line(&mut self.reader, url)?.is_empty() {
                    return Err(HttpError::fatal(format!(
                        "{url}: no line end after a chunk"
                    )));
                }
            },
        }
        Ok(written)
    }
}

/// One line of the response head or of the chunked framing, without its line end.
fn read_line(reader: &mut BufReader<Box<dyn ReadWrite>>, url: &str) -> Result<String, HttpError> {
    let mut line = Vec::new();
    let read = reader
        .by_ref()
        .take(MAX_HEAD as u64)
        .read_until(b'\n', &mut line)
        .map_err(|e| io_error(url, &e))?;
    if read == 0 {
        return Err(HttpError::transient(format!("{url}: connection closed")));
    }
    if line.last() != Some(&b'\n') {
        return Err(HttpError::fatal(format!(
            "{url}: line too long in the response"
        )));
    }
    line.pop();
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line).map_err(|_| HttpError::fatal(format!("{url}: response head not UTF-8")))
}

/// The client settings.
#[derive(Debug, Clone)]
pub(crate) struct Client {
    pub(crate) timeout: Duration,
}

impl Client {
    /// `GET url`, following redirects; from byte `offset` (`Range`) when it is not 0.
    pub(crate) fn get(&self, url: &str, offset: u64) -> Result<Response, HttpError> {
        let mut current = Url::parse(url).map_err(HttpError::fatal)?;
        for _ in 0..=MAX_REDIRECTS {
            let response = self.request(&current, offset)?;
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                let location = response.header("location").ok_or_else(|| {
                    HttpError::fatal(format!(
                        "{current}: HTTP {} without a Location",
                        response.status
                    ))
                })?;
                current = current.join(location).map_err(HttpError::fatal)?;
                continue;
            }
            return Ok(response);
        }
        Err(HttpError::fatal(format!(
            "{url}: more than {MAX_REDIRECTS} redirects"
        )))
    }

    fn connect(&self, url: &Url) -> Result<Box<dyn ReadWrite>, HttpError> {
        let name = url.to_string();
        let addresses = (url.host.as_str(), url.port)
            .to_socket_addrs()
            .map_err(|e| {
                HttpError::transient(format!("{}: cannot resolve {}: {e}", name, url.host))
            })?;
        let mut last = None;
        let mut stream = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, self.timeout) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last = Some(e),
            }
        }
        let stream = stream.ok_or_else(|| {
            HttpError::transient(format!(
                "{name}: cannot connect to {}: {}",
                url.host,
                last.map_or_else(|| "no address".to_owned(), |e| e.to_string())
            ))
        })?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.timeout)))
            .map_err(|e| io_error(&name, &e))?;
        if !url.tls {
            return Ok(Box::new(stream));
        }
        let server = rustls::pki_types::ServerName::try_from(url.host.clone())
            .map_err(|e| HttpError::fatal(format!("{name}: {e}")))?;
        let connection = rustls::ClientConnection::new(tls_config()?, server)
            .map_err(|e| HttpError::fatal(format!("{name}: TLS: {e}")))?;
        Ok(Box::new(rustls::StreamOwned::new(connection, stream)))
    }

    fn request(&self, url: &Url, offset: u64) -> Result<Response, HttpError> {
        let name = url.to_string();
        let mut stream = self.connect(url)?;
        let mut head = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n",
            url.target,
            url.host_header(),
            user_agent()
        );
        if offset > 0 {
            let _ = write!(head, "Range: bytes={offset}-\r\n");
        }
        head.push_str("\r\n");
        stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.flush())
            .map_err(|e| io_error(&name, &e))?;
        let mut reader = BufReader::with_capacity(64 << 10, stream);
        let status_line = read_line(&mut reader, &name)?;
        let mut parts = status_line.splitn(3, ' ');
        let version = parts.next().unwrap_or("");
        let status = parts
            .next()
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|_| version.starts_with("HTTP/1."))
            .ok_or_else(|| HttpError::fatal(format!("{name}: bad status line {status_line:?}")))?;
        let mut headers = Vec::new();
        let mut size = status_line.len();
        loop {
            let line = read_line(&mut reader, &name)?;
            if line.is_empty() {
                break;
            }
            size += line.len();
            if size > MAX_HEAD || headers.len() >= MAX_HEADERS {
                return Err(HttpError::fatal(format!("{name}: response head too large")));
            }
            let (key, value) = line
                .split_once(':')
                .ok_or_else(|| HttpError::fatal(format!("{name}: bad header line {line:?}")))?;
            headers.push((key.trim().to_owned(), value.trim().to_owned()));
        }
        let header = |n: &str| {
            headers
                .iter()
                .find(|(k, _): &&(String, String)| k.eq_ignore_ascii_case(n))
                .map(|(_, v)| v.as_str())
        };
        let framing =
            if header("transfer-encoding")
                .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
            {
                Framing::Chunked
            } else if let Some(length) = header("content-length") {
                Framing::Length(length.parse::<u64>().map_err(|_| {
                    HttpError::fatal(format!("{name}: bad Content-Length {length:?}"))
                })?)
            } else {
                Framing::Close
            };
        Ok(Response {
            status,
            headers,
            body: Body { reader, framing },
            url: url.clone(),
        })
    }
}

/// Whether an HTTP status is worth retrying: too many requests, or a server error that
/// may pass.
pub(crate) const fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

/// The `Retry-After` of a response, in seconds (only the delay form: a date is ignored).
pub(crate) fn retry_after(response: &Response) -> Option<u64> {
    response.header("retry-after")?.trim().parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        let u = Url::parse("https://repo1.maven.org/maven2/a/b.jar").unwrap();
        assert_eq!(
            (u.tls, u.host.as_str(), u.port, u.target.as_str()),
            (true, "repo1.maven.org", 443, "/maven2/a/b.jar")
        );
        assert_eq!(u.to_string(), "https://repo1.maven.org/maven2/a/b.jar");
        let u = Url::parse("https://Example.org:8443?x=1#frag").unwrap();
        assert_eq!(
            (u.host.as_str(), u.port, u.target.as_str()),
            ("example.org", 8443, "/?x=1")
        );
        let u = Url::parse("http://127.0.0.1:9000/m/a").unwrap();
        assert!(!u.tls);
        assert_eq!(u.host_header(), "127.0.0.1:9000");
        let u = Url::parse("http://[::1]:9/x").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("::1", 9));
        assert_eq!(u.to_string(), "http://[::1]:9/x");
        for bad in [
            "ftp://x/y",
            "http://example.org/a",
            "https://user:pw@example.org/",
            "https:///a",
            "https://a b/",
            "https://example.org/a b",
            "https://example.org:99999/",
            "example.org/a",
        ] {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn joins_redirects() {
        let base = Url::parse("https://example.org/a/b/c.zip?q").unwrap();
        assert_eq!(
            base.join("/x/y").unwrap().to_string(),
            "https://example.org/x/y"
        );
        assert_eq!(
            base.join("d.zip").unwrap().to_string(),
            "https://example.org/a/b/d.zip"
        );
        assert_eq!(
            base.join("https://cdn.example.net/z").unwrap().to_string(),
            "https://cdn.example.net/z"
        );
        assert_eq!(
            base.join("//cdn.example.net/z").unwrap().to_string(),
            "https://cdn.example.net/z"
        );
        // Never from HTTPS to plain HTTP, even to a loopback address.
        assert!(base.join("http://127.0.0.1/z").is_err());
        assert!(base.join("http://example.org/z").is_err());
        let local = Url::parse("http://localhost:8/a/b").unwrap();
        assert_eq!(
            local.join("c").unwrap().to_string(),
            "http://localhost:8/a/c"
        );
    }

    #[test]
    fn user_agent_names_the_project() {
        assert!(user_agent().starts_with("einvoice-validator/"));
        assert!(user_agent().contains("https://github.com/einvoice-validator/einvoice-validator"));
    }
}
