//! `endmark view FILE`: the reference review renderer for one file, served
//! to a browser on this computer.
//!
//! This is an example of a host for the renderer, shipped so that one
//! binary gives a complete experience. It is written for one person, one
//! file and one computer, on the standard library alone. It is not a
//! production web server: it answers one request per connection, speaks
//! only as much HTTP/1.1 as its own page needs, and has no TLS and no
//! accounts. A shared review site should host `packages/render` itself.
//!
//! What it serves:
//!
//! - `GET /`: the page, with the token of this run in it.
//! - `GET /NAME`: a file of the page, from the list in `view/assets.rs`.
//! - `GET /api/version`: `{"ok": true, "version"}`. The page polls this.
//! - `GET /api/file`: `{"ok": true, "version", "source", "file", "by",
//!   "for", "read_only"}`.
//! - `POST /api/write`: one operation, as `{"op", "id", "on", "in", "note",
//!   "edits"}`. The server performs it on the file as it is on disk, with
//!   its own author, time and new id, and answers with the new `source`
//!   and `version`, and `id`, `target`, `reanchored`, `detached`, `stale`.
//!
//! A write that is refused, or a file that cannot be read or replaced, is
//! answered with status 200 and `{"ok": false, "code", "message", "hint"}`.
//! A request that this server's own page would never send is answered with
//! a 4xx status and the same JSON.
//!
//! What stands between another web page and the file (see SECURITY.md):
//!
//! - The listener is bound to 127.0.0.1 and nothing else.
//! - `Host` must name this server, which stops DNS rebinding.
//! - Every `/api/` request must carry the token, which is in the page and
//!   nowhere else. A page from another origin cannot read this page.
//! - A write must also be a POST of `application/json` whose `Origin` is
//!   this server.
//! - Every response forbids framing, sniffing and use from another origin,
//!   and carries a Content-Security-Policy that keeps the page to itself.

use std::borrow::Cow;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use endmark_core::write::{self, Stamp, WriteError, Written};
use endmark_core::{Edit, Quote, Verb};
use serde_json::{json, Value};

// The page, built by packages/render/build.ts and committed.
#[rustfmt::skip]
#[path = "../view/assets.rs"]
mod assets;

const DEFAULT_PORT: u16 = 4173;
/// The request line and headers may take this many bytes.
const MAX_HEAD: usize = 16 * 1024;
/// A body may take this many bytes. An operation is a note and a few quotes.
const MAX_BODY: usize = 1024 * 1024;
/// Connections served at once. A browser opens six.
const MAX_CONNECTIONS: usize = 32;
/// How long one read or write may block.
const IO_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a whole request may take to arrive.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

const TOKEN_PLACEHOLDER: &str = "__ENDMARK_TOKEN__";
const TOKEN_HEADER: &str = "x-endmark-token";

/// Sent with every response. The page loads scripts, styles and the wasm
/// module from this server only. `'wasm-unsafe-eval'` lets it compile the
/// wasm module, and inline styles are what mermaid draws diagrams with.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; base-uri 'none'; \
     form-action 'none'; frame-ancestors 'none'";

struct Server {
    /// The file, as named on the command line.
    path: String,
    /// Who writes. `None` when the server is read-only.
    by: Option<String>,
    person: Option<String>,
    port: u16,
    token: String,
    /// The page with the token in it.
    page: Vec<u8>,
    /// Held from reading the file to replacing it, so that two writes
    /// cannot both start from the same text.
    writing: Mutex<()>,
}

pub fn run(args: &[String]) -> Result<ExitCode, String> {
    let mut port = DEFAULT_PORT;
    let mut read_only = false;
    let mut by = std::env::var("ENDMARK_BY").ok();
    // As in the writing commands: the flags win over the environment, and
    // an empty ENDMARK_FOR counts as unset.
    let mut person: Option<String> = std::env::var("ENDMARK_FOR").ok().filter(|p| !p.is_empty());
    let mut files: Vec<&String> = Vec::new();
    let mut only_files = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if only_files || arg == "-" || !arg.starts_with('-') {
            files.push(arg);
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| it.next().cloned())
                .ok_or(format!("{name} needs a value"))
        };
        match name {
            "--" => only_files = true,
            "--read-only" => read_only = true,
            "--port" => {
                port = value()?
                    .parse()
                    .map_err(|_| "--port takes a number from 0 to 65535; 0 picks a free port")?;
            }
            "--by" => by = Some(value()?),
            "--for" => person = Some(value()?),
            _ => return Err(format!("unknown option \"{arg}\"; see endmark --help")),
        }
    }
    let path = match files.as_slice() {
        [file] if *file == "-" => return Err("view needs a file on disk, not standard input".into()),
        [file] => (*file).clone(),
        [] => return Err("view needs a file; see endmark --help".into()),
        _ => return Err("view takes one file".into()),
    };
    let by = by.filter(|b| !b.is_empty());
    if !read_only && by.is_none() {
        return Err(
            "view needs to know who is writing: pass --by HANDLE or set ENDMARK_BY, or pass --read-only".into(),
        );
    }
    // Fail now, not in the browser, when the file is missing or is not text.
    crate::read(&path)?;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|e| {
        if e.kind() == io::ErrorKind::AddrInUse {
            format!("port {port} is in use on 127.0.0.1; pass --port N for another, or --port 0 for any free port")
        } else {
            format!("could not listen on 127.0.0.1:{port}: {e}")
        }
    })?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("could not listen on 127.0.0.1:{port}: {e}"))?
        .port();

    let token = fresh_token();
    let server = Arc::new(Server {
        page: assets::INDEX.replacen(TOKEN_PLACEHOLDER, &token, 1).into_bytes(),
        by: if read_only { None } else { by },
        person: if read_only { None } else { person },
        path,
        port,
        token,
        writing: Mutex::new(()),
    });

    let mut out = io::stdout();
    let name = printable(&server.path);
    let _ = writeln!(out, "Serving {name} at http://127.0.0.1:{port}/ (Ctrl-C to stop)");
    let _ = match (&server.by, &server.person) {
        (None, _) => writeln!(out, "Read-only: the page cannot change {name}."),
        (Some(by), None) => writeln!(out, "The page writes to {name} as {}.", printable(by)),
        (Some(by), Some(person)) => writeln!(
            out,
            "The page writes to {name} as {} for {}.",
            printable(by),
            printable(person)
        ),
    };
    let _ = out.flush();

    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let slot = Slot::take(&active);
        if slot.count > MAX_CONNECTIONS {
            // Too many at once: hang up. The browser tries again.
            continue;
        }
        let server = Arc::clone(&server);
        let _ = std::thread::Builder::new().spawn(move || {
            let _slot = slot;
            serve(&server, stream);
        });
    }
    Ok(ExitCode::SUCCESS)
}

/// One of the connections being served. Dropping it gives the place back.
struct Slot {
    active: Arc<AtomicUsize>,
    count: usize,
}

impl Slot {
    fn take(active: &Arc<AtomicUsize>) -> Slot {
        Slot {
            count: active.fetch_add(1, Ordering::SeqCst) + 1,
            active: Arc::clone(active),
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A file name or a handle with nothing in it that could drive a terminal.
fn printable(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { '?' } else { c }).collect()
}

/// 128 random bits, as 32 hexadecimal digits.
fn fresh_token() -> String {
    let mut bytes = [0u8; 16];
    // The standard library seeds each RandomState from the operating system.
    for half in bytes.chunks_mut(8) {
        let bits = std::collections::hash_map::RandomState::new().build_hasher().finish();
        half.copy_from_slice(&bits.to_le_bytes());
    }
    // Where the system offers its generator as a file, take the token from there as well.
    let mut more = [0u8; 16];
    let from_system = std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut more));
    if from_system.is_ok() {
        for (byte, extra) in bytes.iter_mut().zip(more) {
            *byte ^= extra;
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compares without stopping at the first difference.
fn same_secret(given: &str, expected: &str) -> bool {
    given.len() == expected.len() && given.bytes().zip(expected.bytes()).fold(0, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Names a state of the file, so the page can tell that it changed.
fn version_of(bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(bytes);
    format!("{:016x}-{}", hasher.finish(), bytes.len())
}

// ---- HTTP ----

struct Request {
    method: String,
    /// The request target, as sent.
    target: String,
    /// Names in lower case.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    /// The value of a header that is present exactly once.
    fn header(&self, name: &str) -> Option<&str> {
        let mut found = self.headers.iter().filter(|(n, _)| n == name);
        match (found.next(), found.next()) {
            (Some((_, value)), None) => Some(value),
            _ => None,
        }
    }
}

struct Response {
    status: u16,
    content_type: &'static str,
    gzip: bool,
    allow: Option<&'static str>,
    body: Cow<'static, [u8]>,
}

impl Response {
    fn json(status: u16, value: &Value) -> Response {
        Response {
            status,
            content_type: "application/json; charset=utf-8",
            gzip: false,
            allow: None,
            body: Cow::Owned(value.to_string().into_bytes()),
        }
    }

    /// A request this server's page would not send.
    fn error(status: u16, code: &str, message: &str, hint: &str) -> Response {
        Response::json(
            status,
            &json!({"ok": false, "code": code, "message": message, "hint": hint}),
        )
    }

    fn not_found() -> Response {
        Response::error(404, "not-found", "there is nothing at this address", "")
    }

    fn method_not_allowed(allow: &'static str) -> Response {
        let mut response = Response::error(405, "method-not-allowed", "this address does not take that method", "");
        response.allow = Some(allow);
        response
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    }
}

fn serve(server: &Server, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let response = match read_request(&mut stream) {
        Ok(Some(request)) => respond(server, &request),
        // The connection was opened and closed with nothing sent. Browsers do that.
        Ok(None) => return,
        Err(response) => response,
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\
         Cache-Control: no-store\r\nContent-Security-Policy: {CONTENT_SECURITY_POLICY}\r\n\
         Cross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Resource-Policy: same-origin\r\n\
         Referrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len(),
    );
    if response.gzip {
        head.push_str("Content-Encoding: gzip\r\nVary: Accept-Encoding\r\n");
    }
    if let Some(allow) = response.allow {
        head.push_str(&format!("Allow: {allow}\r\n"));
    }
    head.push_str("\r\n");
    let sent = stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(&response.body))
        .and_then(|()| stream.flush());
    if sent.is_ok() {
        // Say that the response is complete, then let the client finish
        // sending whatever it still had, so that it reads the response and
        // not a reset connection.
        let _ = stream.shutdown(Shutdown::Write);
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let mut rest = [0u8; 4096];
        let mut left = 4 * MAX_BODY;
        while left > 0 {
            match stream.read(&mut rest) {
                Ok(n) if n > 0 => left = left.saturating_sub(n),
                _ => break,
            }
        }
    }
}

/// Reads one request. `Ok(None)` when the client sent nothing at all.
fn read_request(stream: &mut TcpStream) -> Result<Option<Request>, Response> {
    let bad = |message: &str| Response::error(400, "bad-request", message, "");
    let started = Instant::now();
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut read =
        |buffer: &mut Vec<u8>| -> Result<usize, Response> {
            if started.elapsed() > REQUEST_TIMEOUT {
                return Err(Response::error(
                    408,
                    "too-slow",
                    "the request took too long to arrive",
                    "",
                ));
            }
            match stream.read(&mut chunk) {
                Ok(n) => {
                    buffer.extend_from_slice(&chunk[..n]);
                    Ok(n)
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => Err(
                    Response::error(408, "too-slow", "the request took too long to arrive", ""),
                ),
                Err(_) => Err(bad("the request could not be read")),
            }
        };

    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if buffer.len() > MAX_HEAD {
            return Err(Response::error(
                431,
                "too-large",
                "the request headers are too large",
                "",
            ));
        }
        if read(&mut buffer)? == 0 {
            return if buffer.is_empty() {
                Ok(None)
            } else {
                Err(bad("the request ended before its headers did"))
            };
        }
    };
    if head_end > MAX_HEAD {
        return Err(Response::error(
            431,
            "too-large",
            "the request headers are too large",
            "",
        ));
    }
    let head = std::str::from_utf8(&buffer[..head_end]).map_err(|_| bad("the request headers are not text"))?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (method, target, version) = match (first.next(), first.next(), first.next(), first.next()) {
        (Some(method), Some(target), Some(version), None) => (method, target, version),
        _ => return Err(bad("the request line is not METHOD TARGET VERSION")),
    };
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") || method.is_empty() || target.is_empty() {
        return Err(bad("the request line is not HTTP/1.1"));
    }
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(|| bad("a header line has no colon"))?;
        if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c.is_control()) {
            return Err(bad("a header has a malformed name"));
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }
    let mut request = Request {
        method: method.to_string(),
        target: target.to_string(),
        headers,
        body: Vec::new(),
    };

    if request.headers.iter().any(|(name, _)| name == "transfer-encoding") {
        return Err(bad("this server reads a body by its Content-Length only"));
    }
    let lengths = request
        .headers
        .iter()
        .filter(|(name, _)| name == "content-length")
        .count();
    let length = match (lengths, request.header("content-length")) {
        (0, _) if request.method == "POST" => {
            return Err(Response::error(411, "no-length", "a POST needs a Content-Length", ""))
        }
        (0, _) => 0,
        (1, Some(value)) if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
            // A number too long to parse is too large as well.
            value.parse::<usize>().unwrap_or(usize::MAX)
        }
        _ => return Err(bad("the Content-Length is not one number")),
    };
    if length > MAX_BODY {
        return Err(Response::error(413, "too-large", "the request body is too large", ""));
    }
    // Only a POST has a body this server reads.
    if request.method == "POST" {
        let mut body = buffer.split_off(head_end + 4);
        while body.len() < length {
            if read(&mut body)? == 0 {
                return Err(bad("the request ended before its body did"));
            }
        }
        body.truncate(length);
        request.body = body;
    }
    Ok(Some(request))
}

fn respond(server: &Server, request: &Request) -> Response {
    // The address the browser thinks it is talking to must be this server.
    // A page that re-points its own domain name at 127.0.0.1 sends its own
    // name here, and is turned away.
    let Some(host) = request.header("host").map(str::to_ascii_lowercase) else {
        return Response::error(400, "bad-request", "the request needs one Host header", "");
    };
    let named = |name: &str| host == format!("{name}:{}", server.port) || (server.port == 80 && host == name);
    if !named("127.0.0.1") && !named("localhost") {
        return Response::error(
            403,
            "bad-host",
            "this server answers only as 127.0.0.1 or localhost, on its own port",
            "open the address that endmark view printed",
        );
    }

    let path = request.target.split('?').next().unwrap_or_default();
    let get = request.method == "GET";
    match path {
        "/api/version" | "/api/file" | "/api/write" => api(server, request, path, &host),
        "/" | "/index.html" if get => Response {
            status: 200,
            content_type: "text/html; charset=utf-8",
            gzip: false,
            allow: None,
            body: Cow::Owned(server.page.clone()),
        },
        "/" | "/index.html" => Response::method_not_allowed("GET"),
        // A path is looked up in the list of embedded files and nowhere
        // else. Nothing is read from disk by name, so there is no path to
        // escape from.
        _ => match assets::ASSETS.iter().find(|(name, _, _)| *name == path) {
            None => Response::not_found(),
            Some(_) if !get => Response::method_not_allowed("GET"),
            Some(_) if !accepts_gzip(request) => Response::error(
                406,
                "needs-gzip",
                "this file is stored compressed, and the request does not accept gzip",
                "send Accept-Encoding: gzip, as every browser does",
            ),
            Some(&(_, content_type, bytes)) => Response {
                status: 200,
                content_type,
                gzip: true,
                allow: None,
                body: Cow::Borrowed(bytes),
            },
        },
    }
}

fn accepts_gzip(request: &Request) -> bool {
    request
        .headers
        .iter()
        .filter(|(name, _)| name == "accept-encoding")
        .flat_map(|(_, value)| value.split(','))
        .any(|coding| {
            let mut parts = coding.split(';').map(str::trim);
            let name = parts.next().unwrap_or_default().to_ascii_lowercase();
            let refused = parts.any(|p| {
                let q = p.replace(' ', "");
                q.strip_prefix("q=")
                    .is_some_and(|v| v.parse::<f32>().is_ok_and(|v| v == 0.0))
            });
            matches!(name.as_str(), "gzip" | "x-gzip" | "*") && !refused
        })
}

fn api(server: &Server, request: &Request, path: &str, host: &str) -> Response {
    let writes = path == "/api/write";
    let method = if writes { "POST" } else { "GET" };
    if request.method != method {
        return Response::method_not_allowed(method);
    }
    // Browsers say where a request comes from. Only this server's own page may ask.
    if request
        .header("sec-fetch-site")
        .is_some_and(|site| site != "same-origin")
        || request
            .headers
            .iter()
            .filter(|(name, _)| name == "sec-fetch-site")
            .count()
            > 1
    {
        return Response::error(
            403,
            "cross-site",
            "this request comes from another site",
            "use the page that endmark view serves",
        );
    }
    // The token is in the page, which no other origin can read.
    if !request
        .header(TOKEN_HEADER)
        .is_some_and(|given| same_secret(given, &server.token))
    {
        return Response::error(
            403,
            "bad-token",
            "the request does not carry the token of this run of endmark view",
            "reload the page: it may be from an earlier run",
        );
    }
    if !writes {
        return read_api(server, path == "/api/file");
    }

    if request.header("origin").map(str::to_ascii_lowercase) != Some(format!("http://{host}")) {
        return Response::error(
            403,
            "bad-origin",
            "a write must come from this server's own page",
            "use the page that endmark view serves",
        );
    }
    let media = request.header("content-type").unwrap_or_default().split(';').next();
    if media.map(|m| m.trim().to_ascii_lowercase()).as_deref() != Some("application/json") {
        return Response::error(415, "bad-content-type", "a write must be sent as application/json", "");
    }
    let Some(by) = server.by.as_deref() else {
        return Response::error(
            403,
            "read-only",
            "this view is read-only",
            "start endmark view without --read-only to write from the page",
        );
    };
    let operation: Value = match serde_json::from_slice(&request.body) {
        Ok(value @ Value::Object(_)) => value,
        Ok(_) => return Response::error(400, "bad-request", "the body is not a JSON object", ""),
        Err(e) => return Response::error(400, "bad-request", &format!("the body is not JSON: {e}"), ""),
    };
    write_api(server, by, &operation)
}

fn outcome(code: &str, message: &str, hint: &str) -> Response {
    Response::json(
        200,
        &json!({"ok": false, "code": code, "message": message, "hint": hint}),
    )
}

fn read_api(server: &Server, whole: bool) -> Response {
    let hint = "the page shows the file as it was last read";
    let bytes = match std::fs::read(&server.path) {
        Ok(bytes) => bytes,
        Err(e) => return outcome("file-unreadable", &format!("{}: {e}", server.path), hint),
    };
    let version = version_of(&bytes);
    if !whole {
        return Response::json(200, &json!({"ok": true, "version": version}));
    }
    match String::from_utf8(bytes) {
        Ok(source) => Response::json(
            200,
            &json!({
                "ok": true, "version": version, "source": source, "file": server.path,
                "by": server.by, "for": server.person, "read_only": server.by.is_none(),
            }),
        ),
        Err(_) => outcome("file-unreadable", &format!("{}: not UTF-8", server.path), hint),
    }
}

fn write_api(server: &Server, by: &str, operation: &Value) -> Response {
    // One write at a time, from reading the file to replacing it.
    let _writing = server.writing.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let source = match crate::read(&server.path) {
        Ok(source) => source,
        Err(message) => return outcome("file-unreadable", &message, "nothing was written"),
    };
    let at = crate::now_utc();
    let written = match perform(&source, operation, by, server.person.as_deref(), &at) {
        Ok(written) => written,
        Err(Refused::Malformed(message)) => return Response::error(400, "bad-request", &message, ""),
        Err(Refused::ByCore(e)) => return outcome(e.code(), &e.to_string(), &e.hint()),
    };
    // The same step the other commands take: one rename, and not at all
    // if the file changed since it was read (RFC 9.2, 9.5).
    if let Err(message) = crate::replace_file(&server.path, &source, &written.source) {
        return outcome(
            "not-written",
            &message,
            "nothing was written; the page reads the file again, then try once more",
        );
    }
    let op = operation["op"].as_str().unwrap_or_default();
    let _ = writeln!(io::stdout(), "{}: {op} {}", printable(&server.path), written.id);
    Response::json(
        200,
        &json!({
            "ok": true,
            "version": version_of(written.source.as_bytes()),
            "source": written.source,
            "id": written.id,
            "target": written.target,
            "reanchored": written.reanchored,
            "detached": written.detached,
            "stale": written.stale,
        }),
    )
}

enum Refused {
    /// The operation is not one this server takes. Nothing was tried.
    Malformed(String),
    /// The core refused the write.
    ByCore(WriteError),
}

/// Performs one operation on `source`. The operation has the keys the wasm
/// module takes (`crates/endmark-wasm`), without the ones that say who
/// writes and when: the page does not get to choose those.
fn perform(source: &str, operation: &Value, by: &str, person: Option<&str>, at: &str) -> Result<Written, Refused> {
    let malformed = |message: String| Refused::Malformed(message);
    let fields = operation
        .as_object()
        .ok_or_else(|| malformed("the operation is not an object".into()))?;
    for key in fields.keys() {
        if !matches!(key.as_str(), "op" | "id" | "on" | "in" | "note" | "edits") {
            return Err(malformed(format!(
                "the operation has the key \"{key}\"; it takes op, id, on, in, note and edits, and the server sets the rest"
            )));
        }
    }
    let text = |key: &str| match &operation[key] {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.as_str())),
        _ => Err(malformed(format!("\"{key}\" is not a string"))),
    };
    let need = |key: &str| text(key)?.ok_or_else(|| malformed(format!("the operation has no \"{key}\"")));
    let op = need("op")?;
    let stamp = Stamp {
        by,
        at,
        on_behalf_of: person,
        note: text("note")?.unwrap_or(""),
    };
    let quote = text("on")?.map(|on| Quote {
        scope: operation["in"].as_str().map(str::to_string),
        on: on.to_string(),
    });
    text("in")?;
    let edits = || -> Result<Vec<Edit>, Refused> {
        let list = operation["edits"]
            .as_array()
            .ok_or_else(|| malformed("the operation has no list of \"edits\"".into()))?;
        list.iter()
            .map(|edit| {
                let part = |key: &str| edit[key].as_str().map(str::to_string);
                match (part("replace"), part("with")) {
                    (Some(replace), Some(with)) if edit["in"].is_null() || edit["in"].is_string() => Ok(Edit {
                        scope: part("in"),
                        replace,
                        with,
                    }),
                    _ => Err(malformed(
                        "an edit needs \"replace\" and \"with\", and may have \"in\", all strings".into(),
                    )),
                }
            })
            .collect()
    };
    let new_id = |letter| crate::fresh_id(letter, &endmark_core::parse(source));

    let written = match op {
        "comment" => write::comment(source, &new_id('c'), &stamp, quote.as_ref(), None),
        "reply" => write::comment(source, &new_id('c'), &stamp, quote.as_ref(), Some(need("id")?)),
        "suggest" => write::suggest(source, &new_id('s'), &stamp, &edits()?),
        "edit" => write::edit(source, &new_id('e'), &stamp, &edits()?),
        "accept" => write::accept(source, need("id")?, &stamp),
        "anchor" => {
            let quote = quote.ok_or_else(|| malformed("the operation has no \"on\"".into()))?;
            write::anchor(source, need("id")?, &stamp, &quote)
        }
        "revert" => write::revert(source, need("id")?, &new_id('r'), &stamp),
        "reject" | "retract" | "resolve" | "reopen" => match Verb::from_name(op) {
            Some(verb) => write::act(source, verb, need("id")?, &stamp),
            None => return Err(malformed(format!("unknown op \"{op}\""))),
        },
        other => return Err(malformed(format!("unknown op \"{other}\""))),
    };
    written.map_err(Refused::ByCore)
}
