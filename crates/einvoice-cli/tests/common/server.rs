//! A local HTTP/1.1 server for the tests of `rules fetch` (no network): it serves fixed
//! files by path, one request per connection, and records the paths asked for.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The server; it stops when dropped.
pub(crate) struct Server {
    /// `http://127.0.0.1:<port>`: what `EINVOICE_VALIDATOR_MIRROR` takes.
    pub(crate) base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    port: u16,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// Serves `files` (path, content); any other path is a 404.
    pub(crate) fn start(files: BTreeMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (requests, stop) = (Arc::clone(&requests), Arc::clone(&stop));
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        serve(stream, &files, &requests);
                    }
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

    /// The paths asked for so far.
    pub(crate) fn requests(&self) -> Vec<String> {
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

fn serve(stream: TcpStream, files: &BTreeMap<String, Vec<u8>>, requests: &Mutex<Vec<String>>) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split(' ').nth(1).unwrap_or("").to_owned();
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => break,
            Ok(_) if header.trim().is_empty() => break,
            Ok(_) => {}
        }
    }
    requests.lock().unwrap().push(path.clone());
    let mut out = stream;
    let (status, body): (&str, &[u8]) = match files.get(&path) {
        Some(body) => ("200 OK", body),
        None => ("404 Not Found", b""),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(body);
    let _ = out.flush();
    let _ = out.shutdown(std::net::Shutdown::Write);
    let mut sink = Vec::new();
    let _ = out.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = out.read_to_end(&mut sink);
}

/// The path under which a mirror serves `url` (`https://host/path` is `/host/path`).
pub(crate) fn mirror_path(url: &str) -> String {
    format!("/{}", url.split_once("://").map_or(url, |(_, rest)| rest))
}
