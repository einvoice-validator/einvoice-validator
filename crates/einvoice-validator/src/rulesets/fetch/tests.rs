//! The fetcher against a local HTTP server (no network): downloads, digests, resumed
//! downloads, retries, redirects, limits, local directories and earlier downloads.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;

/// A request the server received.
#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub(crate) path: String,
    pub(crate) range: Option<u64>,
    pub(crate) user_agent: Option<String>,
}

/// What the server answers: status, headers, body, and how many bytes of the body to
/// send before closing the connection (`None`: all).
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) cut: Option<usize>,
}

impl Reply {
    pub(crate) fn ok(body: &[u8]) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Length".into(), body.len().to_string())],
            body: body.to_vec(),
            cut: None,
        }
    }

    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            headers: vec![("Content-Length".into(), "0".into())],
            body: Vec::new(),
            cut: None,
        }
    }

    /// The file `body`, from the range the request asks for when `ranges` is set.
    pub(crate) fn file(body: &[u8], request: &Request, ranges: bool) -> Self {
        match request.range {
            Some(from) if ranges => {
                let from = usize::try_from(from).unwrap();
                if from >= body.len() {
                    return Self::status(416);
                }
                let rest = &body[from..];
                Self {
                    status: 206,
                    headers: vec![
                        ("Content-Length".into(), rest.len().to_string()),
                        (
                            "Content-Range".into(),
                            format!("bytes {from}-{}/{}", body.len() - 1, body.len()),
                        ),
                    ],
                    body: rest.to_vec(),
                    cut: None,
                }
            }
            _ => Self::ok(body),
        }
    }
}

type Handler = dyn Fn(&Request, usize) -> Reply + Send + Sync;

/// A local HTTP/1.1 server on a loopback port, answering each request with the
/// handler (given the request and its number, from 0), one request per connection.
pub(crate) struct Server {
    pub(crate) base: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    port: u16,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(
        handler: impl Fn(&Request, usize) -> Reply + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler: Arc<Handler> = Arc::new(handler);
        let thread = {
            let (requests, stop) = (Arc::clone(&requests), Arc::clone(&stop));
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    serve(stream, &*handler, &requests);
                }
            })
        };
        Self {
            base: format!("http://127.0.0.1:{port}"),
            requests,
            stop,
            port,
            thread: Some(thread),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Answers one request, recorded before the answer is sent.
fn serve(stream: TcpStream, handler: &Handler, requests: &Mutex<Vec<Request>>) -> Option<()> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split(' ').nth(1)?.to_owned();
    let mut range = None;
    let mut user_agent = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 || header.trim().is_empty() {
            break;
        }
        let (name, value) = header.trim().split_once(':')?;
        if name.eq_ignore_ascii_case("range") {
            range = value
                .trim()
                .strip_prefix("bytes=")
                .and_then(|r| r.strip_suffix('-'))
                .and_then(|n| n.parse().ok());
        }
        if name.eq_ignore_ascii_case("user-agent") {
            user_agent = Some(value.trim().to_owned());
        }
    }
    let request = Request {
        path,
        range,
        user_agent,
    };
    let count = {
        let mut requests = requests.lock().unwrap();
        requests.push(request.clone());
        requests.len() - 1
    };
    let reply = handler(&request, count);
    let mut out = stream;
    let mut head = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status);
    for (name, value) in &reply.headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    let _ = out.write_all(head.as_bytes());
    let body = reply.cut.map_or(&reply.body[..], |n| &reply.body[..n]);
    let _ = out.write_all(body);
    let _ = out.flush();
    // Let the client read what was sent before the connection goes.
    let _ = out.shutdown(std::net::Shutdown::Write);
    let mut sink = Vec::new();
    let _ = out.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = out.read_to_end(&mut sink);
    Some(())
}

fn temp(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("einvoice-fetch-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn sha256(bytes: &[u8]) -> String {
    digest(Algorithm::Sha256, bytes)
}

fn test_options(server: &Server) -> FetchOptions {
    FetchOptions::new()
        .mirror(&server.base)
        .retry_delay(Duration::from_millis(5))
        .timeout(Duration::from_secs(5))
}

/// Some content of `n` bytes.
fn content(n: usize) -> Vec<u8> {
    (0..n).map(|i| u8::try_from(i % 251).unwrap()).collect()
}

const URL: &str = "https://downloads.example.org/files/data.bin";
const PATH: &str = "/downloads.example.org/files/data.bin";

#[test]
fn downloads_and_checks_the_digest() {
    let data = content(100_000);
    let payload = data.clone();
    let server = Server::start(move |r, _| Reply::file(&payload, r, true));
    let cache = RuleSetCache::at(temp("download"));
    let options = test_options(&server);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(&data));
    assert_eq!(fetcher.obtain(&source).unwrap(), data);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, PATH);
    assert_eq!(requests[0].range, None);
    assert_eq!(
        requests[0].user_agent.as_deref(),
        Some(http::user_agent().as_str())
    );
    // Kept, verified, until the bundles are installed.
    let done = cache.root().join(DOWNLOADS_DIR).join("data.bin");
    assert_eq!(std::fs::read(&done).unwrap(), data);
    // Reused offline: no further request.
    drop(server);
    let offline = FetchOptions::new()
        .mirror("http://127.0.0.1:9")
        .attempts(1)
        .timeout(Duration::from_millis(200));
    let fetcher = Fetcher::new(&cache, &offline).unwrap();
    assert_eq!(fetcher.obtain(&source).unwrap(), data);
    fetcher.remove_downloads(std::slice::from_ref(&source));
    assert!(!done.exists());
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn refuses_a_download_with_another_digest() {
    let data = content(5000);
    let payload = data.clone();
    let server = Server::start(move |r, _| Reply::file(&payload, r, true));
    let cache = RuleSetCache::at(temp("mismatch"));
    let options = test_options(&server);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(b"something else"));
    let error = fetcher.obtain(&source).unwrap_err().to_string();
    assert!(error.contains("refused"), "{error}");
    assert!(error.contains(&sha256(&data)), "{error}");
    // Nothing of it is kept.
    let downloads = cache.root().join(DOWNLOADS_DIR);
    assert_eq!(std::fs::read_dir(&downloads).unwrap().count(), 0);
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn resumes_an_interrupted_download() {
    let data = content(300_000);
    let payload = data.clone();
    // The first answer stops half-way; the next ones honour the range.
    let server = Server::start(move |r, n| {
        if n == 0 {
            let mut reply = Reply::ok(&payload);
            reply.cut = Some(120_000);
            reply
        } else {
            Reply::file(&payload, r, true)
        }
    });
    let cache = RuleSetCache::at(temp("resume"));
    let options = test_options(&server);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(&data));
    assert_eq!(fetcher.obtain(&source).unwrap(), data);
    let ranges: Vec<Option<u64>> = server.requests().iter().map(|r| r.range).collect();
    assert_eq!(ranges, [None, Some(120_000)]);
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn a_part_of_another_file_is_downloaded_again_afresh() {
    let data = content(50_000);
    let payload = data.clone();
    let server = Server::start(move |r, _| Reply::file(&payload, r, true));
    let cache = RuleSetCache::at(temp("stale-part"));
    let downloads = cache.root().join(DOWNLOADS_DIR);
    std::fs::create_dir_all(&downloads).unwrap();
    // An interrupted download of something else, left by an earlier run.
    std::fs::write(downloads.join("data.bin.part"), vec![b'x'; 1000]).unwrap();
    let options = test_options(&server);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(&data));
    assert_eq!(fetcher.obtain(&source).unwrap(), data);
    let ranges: Vec<Option<u64>> = server.requests().iter().map(|r| r.range).collect();
    assert_eq!(ranges, [Some(1000), None]);
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn restarts_when_the_server_cannot_resume() {
    let data = content(40_000);
    let payload = data.clone();
    let server = Server::start(move |r, n| {
        let mut reply = Reply::file(&payload, r, false);
        if n == 0 {
            reply.cut = Some(10_000);
        }
        reply
    });
    let cache = RuleSetCache::at(temp("no-range"));
    let options = test_options(&server);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(&data));
    assert_eq!(fetcher.obtain(&source).unwrap(), data);
    let ranges: Vec<Option<u64>> = server.requests().iter().map(|r| r.range).collect();
    assert_eq!(ranges, [None, Some(10_000)]);
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn retries_on_429_and_5xx_then_gives_up() {
    let data = content(1000);
    let payload = data.clone();
    let server = Server::start(move |r, n| match n {
        0 => Reply::status(503),
        1 => {
            let mut reply = Reply::status(429);
            reply.headers.push(("Retry-After".into(), "0".into()));
            reply
        }
        2 => Reply::status(502),
        _ => Reply::file(&payload, r, true),
    });
    let cache = RuleSetCache::at(temp("retry"));
    let options = test_options(&server);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(&data));
    assert_eq!(fetcher.obtain(&source).unwrap(), data);
    assert_eq!(server.requests().len(), 4);
    drop(server);

    let server = Server::start(|_, _| Reply::status(500));
    let options = test_options(&server).attempts(3);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "other.bin", &sha256(&data));
    let error = fetcher.obtain(&source).unwrap_err().to_string();
    assert!(
        error.contains("HTTP 500") && error.contains("3 attempts"),
        "{error}"
    );
    assert_eq!(server.requests().len(), 3);
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn a_missing_file_is_not_retried() {
    let server = Server::start(|_, _| Reply::status(404));
    let cache = RuleSetCache::at(temp("404"));
    let options = test_options(&server);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(b""));
    let error = fetcher.obtain(&source).unwrap_err().to_string();
    assert!(error.contains("HTTP 404"), "{error}");
    assert_eq!(server.requests().len(), 1);
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn follows_redirects_within_a_limit() {
    let data = content(2000);
    let payload = data.clone();
    let server = Server::start(move |r, _| {
        if r.path == PATH {
            let mut reply = Reply::status(302);
            reply
                .headers
                .push(("Location".into(), "/elsewhere/data.bin".into()));
            reply
        } else if r.path == "/elsewhere/data.bin" {
            Reply::file(&payload, r, true)
        } else {
            // Loops forever.
            let mut reply = Reply::status(301);
            reply.headers.push(("Location".into(), r.path.clone()));
            reply
        }
    });
    let cache = RuleSetCache::at(temp("redirect"));
    let options = test_options(&server);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(&data));
    assert_eq!(fetcher.obtain(&source).unwrap(), data);
    let source = Source::for_test("https://loop.example.org/x", "loop.bin", &sha256(&data));
    let error = fetcher.obtain(&source).unwrap_err().to_string();
    assert!(error.contains("redirects"), "{error}");
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn reads_chunked_bodies() {
    let data = content(70_000);
    let payload = data.clone();
    let server = Server::start(move |_, _| {
        let mut body = Vec::new();
        for chunk in payload.chunks(30_000) {
            body.extend(format!("{:x};ext=1\r\n", chunk.len()).as_bytes());
            body.extend(chunk);
            body.extend(b"\r\n");
        }
        body.extend(b"0\r\nX-Trailer: y\r\n\r\n");
        Reply {
            status: 200,
            headers: vec![("Transfer-Encoding".into(), "chunked".into())],
            body,
            cut: None,
        }
    });
    let cache = RuleSetCache::at(temp("chunked"));
    let options = test_options(&server);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(&data));
    assert_eq!(fetcher.obtain(&source).unwrap(), data);
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn refuses_downloads_beyond_the_size_limit() {
    let data = content(10_000);
    let payload = data.clone();
    let server = Server::start(move |r, n| {
        if n == 0 {
            Reply::file(&payload, r, true)
        } else {
            // No length announced: stopped while reading.
            Reply {
                status: 200,
                headers: Vec::new(),
                body: payload.clone(),
                cut: None,
            }
        }
    });
    let cache = RuleSetCache::at(temp("limit"));
    let options = test_options(&server).max_size(9_999);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(&data));
    let error = fetcher.obtain(&source).unwrap_err().to_string();
    assert!(error.contains("limit of 9999 bytes"), "{error}");
    let error = fetcher.obtain(&source).unwrap_err().to_string();
    assert!(error.contains("limit of 9999 bytes"), "{error}");
    // Not retried.
    assert_eq!(server.requests().len(), 2);
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn takes_files_from_a_local_directory_checked() {
    let data = content(3000);
    let dir = temp("from");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("data.bin"), &data).unwrap();
    let cache = RuleSetCache::at(temp("from-cache"));
    // No server: nothing may be downloaded.
    let options = FetchOptions::new()
        .from_dir(&dir)
        .mirror("http://127.0.0.1:9")
        .attempts(1);
    let fetcher = Fetcher::new(&cache, &options).unwrap();
    let source = Source::for_test(URL, "data.bin", &sha256(&data));
    assert_eq!(fetcher.obtain(&source).unwrap(), data);
    let source = Source::for_test(URL, "data.bin", &sha256(b"other"));
    let error = fetcher.obtain(&source).unwrap_err().to_string();
    assert!(error.contains("data.bin has SHA-256"), "{error}");
    // The local file is never removed.
    fetcher.remove_downloads(std::slice::from_ref(&source));
    assert!(dir.join("data.bin").exists());
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn installs_a_release_in_one_rename() {
    let cache = RuleSetCache::at(temp("install"));
    let files = vec![
        ("a/bundle.toml".to_owned(), b"one".to_vec()),
        ("b/x/y.xslt".to_owned(), b"two".to_vec()),
    ];
    cache.install("rel-1", &files).unwrap();
    let release = cache.root().join(RULES_DIR).join("rel-1");
    assert_eq!(
        std::fs::read(release.join("a/bundle.toml")).unwrap(),
        b"one"
    );
    // Again: replaced, nothing of the old one or of the staging left.
    cache
        .install("rel-1", &[("c.txt".to_owned(), b"three".to_vec())])
        .unwrap();
    assert!(!release.join("a").exists());
    assert_eq!(std::fs::read(release.join("c.txt")).unwrap(), b"three");
    let names: Vec<String> = std::fs::read_dir(cache.root().join(RULES_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["rel-1"]);
    std::fs::remove_dir_all(cache.root()).unwrap();
}

#[test]
fn an_unknown_name_is_refused() {
    let cache = RuleSetCache::at(temp("unknown"));
    let error = cache.fetch("nope", &FetchOptions::new()).unwrap_err();
    assert!(matches!(error, Error::Bundle { .. }), "{error:?}");
    assert!(error.to_string().contains("factur-x-en16931"), "{error}");
    assert!(!cache.root().exists());
}

#[test]
fn fetch_refuses_a_family_archive_that_is_not_the_pinned_one() {
    // A server that serves a ZIP for every URL: its digest is not the pinned one.
    let zip = archive::tests::zip(&[("a.txt", b"a", false)]);
    let server = Server::start(move |r, _| Reply::file(&zip, r, true));
    let cache = RuleSetCache::at(temp("family"));
    let error = cache
        .fetch("factur-x", &test_options(&server))
        .unwrap_err()
        .to_string();
    assert!(error.contains("refused"), "{error}");
    let r = catalogue::fetched_rule_set("factur-x-en16931", None)
        .unwrap()
        .unwrap();
    assert_eq!(cache.status(r), CacheStatus::Missing);
    std::fs::remove_dir_all(cache.root()).unwrap();
}
