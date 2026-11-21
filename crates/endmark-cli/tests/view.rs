//! `endmark view` serves one file to a browser on this computer. These
//! tests start the real binary on a free port and talk to it over a
//! socket, as a browser and as a page that should be turned away.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A directory of its own for one test, removed afterwards.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("endmark-view-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    /// A copy of the worked example under this name.
    fn example(&self, name: &str) -> PathBuf {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/worked-example.md");
        let file = self.0.join(name);
        std::fs::copy(fixture, &file).unwrap();
        file
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn endmark(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_endmark"))
        .args(args)
        .env_remove("ENDMARK_BY")
        .stdin(Stdio::null())
        .output()
        .expect("run endmark")
}

/// A running `endmark view`. Dropping it stops the server.
struct Server {
    child: Child,
    port: u16,
    /// The first line it printed.
    banner: String,
}

impl Server {
    fn start(file: &Path, options: &[&str]) -> Server {
        Server::start_with(file, options, &[])
    }

    /// Starts it with these environment variables set.
    fn start_with(file: &Path, options: &[&str], env: &[(&str, &str)]) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_endmark"))
            .arg("view")
            .args(["--port", "0"])
            .args(options)
            .arg(file)
            .env_remove("ENDMARK_BY")
            .env_remove("ENDMARK_FOR")
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run endmark view");
        let mut banner = String::new();
        BufReader::new(child.stdout.take().expect("stdout"))
            .read_line(&mut banner)
            .expect("the first line");
        let port = banner
            .split("http://127.0.0.1:")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .and_then(|port| port.parse().ok())
            .unwrap_or_else(|| {
                let mut stderr = String::new();
                let _ = child.stderr.take().map(|mut e| e.read_to_string(&mut stderr));
                panic!("endmark view did not say where it serves: {banner:?} {stderr:?}")
            });
        Server {
            child,
            port,
            banner: banner.trim_end().to_string(),
        }
    }

    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Sends bytes as they are and returns everything the server answers.
    fn raw(&self, request: &[u8]) -> Reply {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        stream.write_all(request).expect("send");
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).expect("the server answers and closes");
        Reply::parse(&bytes)
    }

    /// A request with a correct Host, as a browser would send it.
    fn request(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Reply {
        let mut text = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\n", self.host());
        for (name, value) in headers {
            text.push_str(&format!("{name}: {value}\r\n"));
        }
        if method == "POST" {
            text.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        text.push_str("\r\n");
        let mut bytes = text.into_bytes();
        bytes.extend_from_slice(body);
        self.raw(&bytes)
    }

    fn get(&self, path: &str, headers: &[(&str, &str)]) -> Reply {
        self.request("GET", path, headers, b"")
    }

    /// The token, read from the page as the page's own script reads it.
    fn token(&self) -> String {
        let page = self.get("/", &[]).text();
        let token = page
            .split("<meta name=\"endmark-token\" content=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("the page has the token");
        assert_eq!(token.len(), 32, "{token}");
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
        token.to_string()
    }

    fn api(&self, path: &str) -> Value {
        let reply = self.get(path, &[("X-Endmark-Token", &self.token())]);
        assert_eq!(reply.status, 200, "{}", reply.text());
        reply.json()
    }

    /// A write as the page sends it.
    fn write(&self, operation: &Value) -> Reply {
        let origin = format!("http://{}", self.host());
        self.request(
            "POST",
            "/api/write",
            &[
                ("X-Endmark-Token", &self.token()),
                ("Origin", &origin),
                ("Content-Type", "application/json"),
                ("Sec-Fetch-Site", "same-origin"),
            ],
            operation.to_string().as_bytes(),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn parse(bytes: &[u8]) -> Reply {
        let end = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or_else(|| panic!("not an HTTP response: {:?}", String::from_utf8_lossy(bytes)));
        let head = std::str::from_utf8(&bytes[..end]).expect("headers are text");
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .and_then(|line| line.strip_prefix("HTTP/1.1 "))
            .and_then(|rest| rest.split(' ').next())
            .and_then(|code| code.parse().ok())
            .expect("a status line");
        let headers = lines
            .map(|line| {
                let (name, value) = line.split_once(": ").expect("a header line");
                (name.to_ascii_lowercase(), value.to_string())
            })
            .collect();
        let reply = Reply {
            status,
            headers,
            body: bytes[end + 4..].to_vec(),
        };
        assert_eq!(
            reply.header("content-length"),
            Some(reply.body.len().to_string().as_str()),
            "the length is the length of the body"
        );
        assert_eq!(reply.header("connection"), Some("close"));
        reply
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("a text body")
    }

    fn json(&self) -> Value {
        assert_eq!(self.header("content-type"), Some("application/json; charset=utf-8"));
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.text()))
    }

    /// A 4xx answer with a code, and never the file.
    fn refused(&self, status: u16, code: &str) {
        assert_eq!(self.status, status, "{}", self.text());
        let json = self.json();
        assert_eq!(json["ok"], false);
        assert_eq!(json["code"], code, "{json}");
        assert!(json.get("source").is_none());
    }
}

#[test]
fn it_says_where_it_serves() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    let server = Server::start(&file, &["--by", "adam"]);
    assert_eq!(
        server.banner,
        format!(
            "Serving {} at http://127.0.0.1:{}/ (Ctrl-C to stop)",
            file.display(),
            server.port
        )
    );
}

#[test]
fn it_takes_the_person_from_endmark_for_as_the_commands_do() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    let env = [("ENDMARK_BY", "agent:claude"), ("ENDMARK_FOR", "adam")];
    let from_env = Server::start_with(&file, &[], &env);
    let state = from_env.api("/api/file");
    assert_eq!(
        (state["by"].as_str(), state["for"].as_str()),
        (Some("agent:claude"), Some("adam"))
    );
    drop(from_env);

    let flag_wins = Server::start_with(&file, &["--for", "beth"], &env);
    assert_eq!(flag_wins.api("/api/file")["for"], "beth");
    drop(flag_wins);

    let empty = Server::start_with(&file, &["--by", "maria"], &[("ENDMARK_FOR", "")]);
    assert_eq!(empty.api("/api/file")["for"], Value::Null);
}

#[test]
fn it_serves_the_page_and_every_file_of_it() {
    let dir = TempDir::new();
    let server = Server::start(&dir.example("notes.md"), &["--by", "adam"]);

    let page = server.get("/", &[]);
    assert_eq!(page.status, 200);
    assert_eq!(page.header("content-type"), Some("text/html; charset=utf-8"));
    assert_eq!(page.header("content-encoding"), None);
    let html = page.text();
    assert!(html.starts_with("<!doctype html>"));
    assert!(!html.contains("__ENDMARK_TOKEN__"), "the token is filled in");
    assert!(
        !html.contains("Relay"),
        "the page does not hold the document; it asks for it"
    );
    assert_eq!(server.get("/index.html", &[]).text(), html);
    // The token is the same for the whole run.
    assert_eq!(server.token(), server.token());

    // Every response keeps the page to itself.
    for reply in [&page, &server.get("/nothing-here", &[]), &server.get("/api/file", &[])] {
        let policy = reply.header("content-security-policy").expect("a policy");
        assert!(policy.starts_with("default-src 'self';"), "{policy}");
        assert!(policy.contains("frame-ancestors 'none'"));
        assert!(!policy.contains("http") && !policy.contains('*'), "{policy}");
        assert_eq!(reply.header("x-frame-options"), Some("DENY"));
        assert_eq!(reply.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(reply.header("cross-origin-resource-policy"), Some("same-origin"));
        assert_eq!(reply.header("referrer-policy"), Some("no-referrer"));
        assert_eq!(reply.header("cache-control"), Some("no-store"));
        assert_eq!(reply.header("access-control-allow-origin"), None);
    }

    // The script and the stylesheet the page names, and the wasm module.
    let mut wanted: Vec<(String, &str)> = vec![("/endmark.wasm".to_string(), "application/wasm")];
    for (attribute, content_type) in [
        ("src=\"./", "text/javascript; charset=utf-8"),
        ("href=\"./", "text/css; charset=utf-8"),
    ] {
        let name = html
            .split(attribute)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("the page names a script and a stylesheet");
        wanted.push((format!("/{name}"), content_type));
    }
    assert_eq!(wanted.len(), 3);
    for (path, content_type) in &wanted {
        let reply = server.get(path, &[("Accept-Encoding", "gzip, deflate")]);
        assert_eq!(reply.status, 200, "{path}");
        assert_eq!(reply.header("content-type"), Some(*content_type), "{path}");
        assert_eq!(reply.header("content-encoding"), Some("gzip"), "{path}");
        assert_eq!(reply.header("vary"), Some("Accept-Encoding"), "{path}");
        assert_eq!(&reply.body[..2], &[0x1f, 0x8b], "{path} is gzip");
        // A client that cannot read gzip is told so, and is not sent bytes it would misread.
        server.get(path, &[]).refused(406, "needs-gzip");
        server
            .get(path, &[("Accept-Encoding", "gzip;q=0, br")])
            .refused(406, "needs-gzip");
        server
            .request("POST", path, &[], b"")
            .refused(405, "method-not-allowed");
    }
    server.request("POST", "/", &[], b"").refused(405, "method-not-allowed");
    server
        .request("OPTIONS", "/api/write", &[], b"")
        .refused(405, "method-not-allowed");
}

#[test]
fn it_answers_only_to_its_own_name() {
    let dir = TempDir::new();
    let server = Server::start(&dir.example("notes.md"), &["--by", "adam"]);
    let token = server.token();
    let ask = |host_line: &str| {
        server.raw(format!("GET /api/file HTTP/1.1\r\n{host_line}X-Endmark-Token: {token}\r\n\r\n").as_bytes())
    };

    assert_eq!(ask(&format!("Host: 127.0.0.1:{}\r\n", server.port)).status, 200);
    assert_eq!(ask(&format!("Host: localhost:{}\r\n", server.port)).status, 200);
    assert_eq!(ask(&format!("Host: LOCALHOST:{}\r\n", server.port)).status, 200);

    // A name that was re-pointed at 127.0.0.1, with the right port and the right token.
    ask(&format!("Host: attacker.example:{}\r\n", server.port)).refused(403, "bad-host");
    ask("Host: attacker.example\r\n").refused(403, "bad-host");
    ask("Host: 127.0.0.1\r\n").refused(403, "bad-host");
    ask(&format!("Host: 127.0.0.1:{}\r\n", server.port.wrapping_add(1))).refused(403, "bad-host");
    ask(&format!("Host: 127.0.0.1.attacker.example:{}\r\n", server.port)).refused(403, "bad-host");
    ask(&format!("Host: [::1]:{}\r\n", server.port)).refused(403, "bad-host");
    ask("").refused(400, "bad-request");
    ask(&format!(
        "Host: 127.0.0.1:{0}\r\nHost: attacker.example:{0}\r\n",
        server.port
    ))
    .refused(400, "bad-request");
    // The page itself is not served under another name either.
    let page = server.raw(b"GET / HTTP/1.1\r\nHost: attacker.example\r\n\r\n");
    page.refused(403, "bad-host");
    assert!(!page.text().contains(&token));
}

#[test]
fn reading_the_file_needs_the_token() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    let server = Server::start(&file, &["--by", "agent:claude", "--for", "adam"]);
    let source = std::fs::read_to_string(&file).unwrap();

    for path in ["/api/file", "/api/version"] {
        server.get(path, &[]).refused(403, "bad-token");
        server.get(path, &[("X-Endmark-Token", "")]).refused(403, "bad-token");
        server
            .get(path, &[("X-Endmark-Token", &"0".repeat(32))])
            .refused(403, "bad-token");
        // The right token, from a page of another site.
        server
            .get(
                path,
                &[("X-Endmark-Token", &server.token()), ("Sec-Fetch-Site", "cross-site")],
            )
            .refused(403, "cross-site");
        server
            .request("POST", path, &[("X-Endmark-Token", &server.token())], b"{}")
            .refused(405, "method-not-allowed");
    }

    let state = server.api("/api/file");
    assert_eq!(state["ok"], true);
    assert_eq!(state["source"], source);
    assert_eq!(state["file"], file.to_string_lossy().as_ref());
    assert_eq!(state["by"], "agent:claude");
    assert_eq!(state["for"], "adam");
    assert_eq!(state["read_only"], false);
    assert_eq!(
        server.api("/api/version"),
        json!({"ok": true, "version": state["version"]})
    );
}

#[test]
fn a_write_that_is_not_from_the_page_is_turned_away() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    let server = Server::start(&file, &["--by", "adam"]);
    let before = std::fs::read(&file).unwrap();
    let token = server.token();
    let origin = format!("http://{}", server.host());
    let body = json!({"op": "accept", "id": "s-4q8d"}).to_string();
    let post = |headers: &[(&str, &str)]| server.request("POST", "/api/write", headers, body.as_bytes());
    let json_type = ("Content-Type", "application/json");

    // No token, and a wrong one.
    post(&[("Origin", &origin), json_type]).refused(403, "bad-token");
    post(&[("X-Endmark-Token", &"f".repeat(32)), ("Origin", &origin), json_type]).refused(403, "bad-token");
    post(&[("X-Endmark-Token", &token[..31]), ("Origin", &origin), json_type]).refused(403, "bad-token");
    // The token in the address or in the body does not count.
    server
        .request(
            "POST",
            &format!("/api/write?token={token}"),
            &[("Origin", &origin), json_type],
            body.as_bytes(),
        )
        .refused(403, "bad-token");
    // Another origin, or none.
    for other in [
        "http://attacker.example",
        "null",
        "https://127.0.0.1",
        &format!("https://{}", server.host()),
        &format!("http://127.0.0.1:{}", server.port.wrapping_add(1)),
    ] {
        post(&[("X-Endmark-Token", &token), ("Origin", other), json_type]).refused(403, "bad-origin");
    }
    post(&[("X-Endmark-Token", &token), json_type]).refused(403, "bad-origin");
    // What the browser itself says about where the request comes from.
    for site in ["cross-site", "same-site", "none"] {
        post(&[
            ("X-Endmark-Token", &token),
            ("Origin", &origin),
            json_type,
            ("Sec-Fetch-Site", site),
        ])
        .refused(403, "cross-site");
    }
    // The content types a form on another site can send.
    for content_type in [
        "text/plain",
        "application/x-www-form-urlencoded",
        "multipart/form-data",
        "application/jsonp",
    ] {
        post(&[
            ("X-Endmark-Token", &token),
            ("Origin", &origin),
            ("Content-Type", content_type),
        ])
        .refused(415, "bad-content-type");
    }
    post(&[("X-Endmark-Token", &token), ("Origin", &origin)]).refused(415, "bad-content-type");
    // A GET changes nothing, whatever it carries.
    let get = server.get(
        "/api/write",
        &[("X-Endmark-Token", &token), ("Origin", &origin), json_type],
    );
    get.refused(405, "method-not-allowed");
    assert_eq!(get.header("allow"), Some("POST"));

    assert_eq!(std::fs::read(&file).unwrap(), before, "nothing was written");
    // And the same request with everything in place is taken.
    let done = server.write(&json!({"op": "accept", "id": "s-4q8d"}));
    assert_eq!(done.status, 200);
    assert_eq!(done.json()["ok"], true, "{}", done.text());
    assert_ne!(std::fs::read(&file).unwrap(), before);
}

#[test]
fn read_only_shows_the_file_and_refuses_every_write() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    // No --by: nothing will be written, so nobody needs to be named.
    let server = Server::start(&file, &["--read-only"]);
    let before = std::fs::read(&file).unwrap();

    let state = server.api("/api/file");
    assert_eq!(state["read_only"], true);
    assert_eq!(state["by"], Value::Null);
    assert_eq!(state["source"].as_str().map(str::as_bytes), Some(&before[..]));

    for operation in [
        json!({"op": "accept", "id": "s-4q8d"}),
        json!({"op": "comment", "note": "Hello."}),
        json!({"op": "resolve", "id": "c-9d1t"}),
    ] {
        server.write(&operation).refused(403, "read-only");
    }
    assert_eq!(std::fs::read(&file).unwrap(), before);

    // Naming an author does not make it writable.
    let named = Server::start(&file, &["--read-only", "--by", "adam"]);
    assert_eq!(named.api("/api/file")["by"], Value::Null);
    named
        .write(&json!({"op": "accept", "id": "s-4q8d"}))
        .refused(403, "read-only");
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

/// Each write from the page leaves the file exactly as the matching
/// command would: the same operation is replayed with the CLI on a second
/// copy, with the id and the time the server chose.
#[test]
fn a_write_changes_the_file_as_the_command_would() {
    let dir = TempDir::new();
    let served = dir.example("served.md");
    let mirror = dir.example("mirror.md");
    let server = Server::start(&served, &["--by", "agent:claude", "--for", "adam"]);

    let mut ids: Vec<String> = Vec::new();
    // The operation, then the command line that matches it. `NEW` stands
    // for the id the server gave the new item, `#n` for the id step n made.
    let steps: Vec<(Value, Vec<&str>)> = vec![
        (
            json!({"op": "comment", "on": "webhook events", "note": "Which events?"}),
            vec![
                "comment",
                "--id",
                "NEW",
                "--on",
                "webhook events",
                "--note",
                "Which events?",
                "FILE",
            ],
        ),
        (
            json!({"op": "comment", "on": "relay", "in": "1. Install the `relay` binary.", "note": "Which binary?"}),
            vec![
                "comment",
                "--id",
                "NEW",
                "--in",
                "1. Install the `relay` binary.",
                "--on",
                "relay",
                "--note",
                "Which binary?",
                "FILE",
            ],
        ),
        (
            json!({"op": "reply", "id": "c-9d1t", "note": "Yes.\n\nAs `RELAY_RETRIES`."}),
            vec![
                "reply",
                "--id",
                "NEW",
                "--note",
                "Yes.\n\nAs `RELAY_RETRIES`.",
                "FILE",
                "c-9d1t",
            ],
        ),
        (
            json!({"op": "suggest", "note": "Give slow consumers time.", "edits": [
                {"replace": "5 second", "with": "10 second"},
                {"in": "2. Set `RELAY_TOKEN`.", "replace": "Set", "with": "Export"},
            ]}),
            vec![
                "suggest",
                "--id",
                "NEW",
                "--note",
                "Give slow consumers time.",
                "--replace",
                "5 second",
                "--with",
                "10 second",
                "--in",
                "2. Set `RELAY_TOKEN`.",
                "--replace",
                "Set",
                "--with",
                "Export",
                "FILE",
            ],
        ),
        (
            json!({"op": "accept", "id": "s-4q8d"}),
            vec!["accept", "FILE", "s-4q8d"],
        ),
        (
            json!({"op": "revert", "id": "s-4q8d"}),
            vec!["revert", "--id", "NEW", "FILE", "s-4q8d"],
        ),
        // Restoring: the page reverts the revert.
        (
            json!({"op": "revert", "id": "#5"}),
            vec!["revert", "--id", "NEW", "FILE", "#5"],
        ),
        (
            json!({"op": "resolve", "id": "c-9d1t"}),
            vec!["resolve", "FILE", "c-9d1t"],
        ),
        (
            json!({"op": "reopen", "id": "c-9d1t"}),
            vec!["reopen", "FILE", "c-9d1t"],
        ),
        (json!({"op": "reject", "id": "#3"}), vec!["reject", "FILE", "#3"]),
        (json!({"op": "retract", "id": "#0"}), vec!["retract", "FILE", "#0"]),
    ];
    let count = steps.len();
    for (n, (mut operation, command)) in steps.into_iter().enumerate() {
        let earlier = |text: &str| match text.strip_prefix('#') {
            Some(step) => ids[step.parse::<usize>().unwrap()].clone(),
            None => text.to_string(),
        };
        if let Some(id) = operation["id"].as_str().map(earlier) {
            operation["id"] = json!(id);
        }
        let reply = server.write(&operation);
        assert_eq!(reply.status, 200, "{operation}: {}", reply.text());
        let done = reply.json();
        assert_eq!(done["ok"], true, "{operation}: {done}");
        let after = std::fs::read_to_string(&served).unwrap();
        assert_eq!(done["source"], after, "the answer is the file as written");
        assert_eq!(done["version"], server.api("/api/version")["version"]);
        let id = done["id"].as_str().unwrap().to_string();
        // The server stamps the time. Every record of one write has the same one.
        let at = after
            .rsplit(" at=")
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            .expect("the last record has a time")
            .to_string();

        let mirror_path = mirror.to_string_lossy().into_owned();
        let mut args: Vec<String> = vec![command[0].to_string()];
        args.extend(["--by", "agent:claude", "--for", "adam", "--at", &at].map(String::from));
        for word in &command[1..] {
            args.push(match *word {
                "NEW" => id.clone(),
                "FILE" => mirror_path.clone(),
                other => earlier(other),
            });
        }
        let out = endmark(&args.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            after,
            std::fs::read_to_string(&mirror).unwrap(),
            "step {n}, {operation}: the page and the command wrote different files"
        );
        ids.push(id);
    }
    assert_eq!(ids.len(), count);
    // What the server stamped is what it was started with, on every record it wrote.
    let written = std::fs::read_to_string(&served).unwrap();
    let original = std::fs::read_to_string(dir.example("original.md")).unwrap();
    let records = |text: &str| text.matches("<!--endmark ").count();
    let stamped = |text: &str| text.matches(" by=agent:claude for=adam at=").count();
    let added = records(&written) - records(&original);
    assert!(added >= count, "at least one record per write");
    assert_eq!(
        stamped(&written) - stamped(&original),
        added,
        "every record the server wrote names the author it was started with"
    );
    let lint = endmark(&["lint", "--strict", &served.to_string_lossy()]);
    assert_eq!(lint.status.code(), Some(0), "{}", String::from_utf8_lossy(&lint.stdout));
}

#[test]
fn a_refused_write_says_why_and_leaves_the_file_alone() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    let server = Server::start(&file, &["--by", "adam"]);
    let before = std::fs::read(&file).unwrap();

    // Refused by the core: status 200, and the code, message and hint the command would print.
    for (operation, code) in [
        (json!({"op": "retract", "id": "c-7f3k"}), "not-author"),
        (
            json!({"op": "comment", "on": "no such text", "note": "?"}),
            "quote-not-found",
        ),
        (json!({"op": "comment", "on": "relay", "note": "?"}), "quote-ambiguous"),
        (json!({"op": "accept", "id": "s-0000"}), "not-found"),
        (json!({"op": "reopen", "id": "c-9d1t"}), "no-change"),
        (json!({"op": "comment"}), "empty"),
    ] {
        let reply = server.write(&operation);
        assert_eq!(reply.status, 200, "{operation}");
        let refusal = reply.json();
        assert_eq!(refusal["ok"], false, "{operation}");
        assert_eq!(refusal["code"], code, "{operation}: {refusal}");
        assert!(!refusal["message"].as_str().unwrap().is_empty());
        assert!(!refusal["hint"].as_str().unwrap().is_empty(), "{refusal}");
        assert!(refusal.get("source").is_none());
    }
    // The same words as the command.
    let said = server.write(&json!({"op": "retract", "id": "c-7f3k"})).json();
    let out = endmark(&["retract", "--by", "adam", &file.to_string_lossy(), "c-7f3k"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains(said["message"].as_str().unwrap()), "{stderr}");
    assert!(stderr.contains(said["hint"].as_str().unwrap()), "{stderr}");

    // Not an operation at all: a 400. The page does not choose who writes or when.
    for operation in [
        json!({"op": "accept", "id": "s-4q8d", "by": "maria"}),
        json!({"op": "accept", "id": "s-4q8d", "at": "2020-01-01T00:00:00Z"}),
        json!({"op": "comment", "note": "x", "new_id": "c-mine"}),
        json!({"op": "comment", "note": "x", "for": "maria"}),
        json!({"op": "accept", "id": "s-4q8d", "source": "Replaced.\n"}),
        json!({"op": "status"}),
        json!({"op": "frobnicate", "id": "s-4q8d"}),
        json!({"op": "accept"}),
        json!({"op": "accept", "id": 7}),
        json!({"op": "suggest"}),
        json!({"op": "suggest", "edits": [{"replace": "port 8080"}]}),
        json!({"id": "s-4q8d"}),
        json!(["accept", "s-4q8d"]),
        json!("accept"),
    ] {
        server.write(&operation).refused(400, "bad-request");
    }
    let origin = format!("http://{}", server.host());
    server
        .request(
            "POST",
            "/api/write",
            &[
                ("X-Endmark-Token", &server.token()),
                ("Origin", &origin),
                ("Content-Type", "application/json; charset=utf-8"),
            ],
            b"{\"op\": \"accept\", ",
        )
        .refused(400, "bad-request");

    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[test]
fn the_page_can_ask_whether_the_file_changed() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    let server = Server::start(&file, &["--by", "adam"]);
    let first = server.api("/api/version")["version"].clone();
    assert_eq!(
        server.api("/api/version")["version"],
        first,
        "unchanged file, same version"
    );

    // Another terminal writes a comment.
    let out = endmark(&[
        "comment",
        "--by",
        "maria",
        "--on",
        "webhook events",
        "--note",
        "From elsewhere.",
        &file.to_string_lossy(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    let second = server.api("/api/version")["version"].clone();
    assert_ne!(second, first);
    let state = server.api("/api/file");
    assert_eq!(state["version"], second);
    assert!(state["source"].as_str().unwrap().contains("From elsewhere."));

    // An editor changes one letter and keeps the length.
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.replace("port 8080 by default", "port 9090 by default")).unwrap();
    let third = server.api("/api/version")["version"].clone();
    assert!(third != second && third != first);

    // A write from the page goes onto the file as it is now, not as the page last saw it.
    let done = server.write(&json!({"op": "resolve", "id": "c-9d1t"})).json();
    assert_eq!(done["ok"], true, "{done}");
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains("port 9090 by default") && written.contains("From elsewhere."));
    assert!(written.contains("<!--endmark resolve c-9d1t by=adam "));

    // The file goes away, and comes back. The server says so and keeps running.
    std::fs::write(&file, [0xff, 0xfe, b'\n']).unwrap();
    assert_eq!(server.api("/api/file")["code"], "file-unreadable");
    assert_eq!(
        server.write(&json!({"op": "reopen", "id": "c-9d1t"})).json()["code"],
        "file-unreadable"
    );
    std::fs::remove_file(&file).unwrap();
    for path in ["/api/version", "/api/file"] {
        let gone = server.api(path);
        assert_eq!(gone["ok"], false);
        assert_eq!(gone["code"], "file-unreadable");
    }
    std::fs::write(&file, &written).unwrap();
    assert_eq!(server.api("/api/file")["source"], written);
}

#[test]
fn requests_that_are_too_large_or_malformed_get_an_answer() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    let server = Server::start(&file, &["--by", "adam"]);
    let before = std::fs::read(&file).unwrap();
    let host = format!("Host: {}\r\n", server.host());
    let token = server.token();
    let origin = format!("http://{}", server.host());

    // A body larger than the server takes: refused from its length, before it is read.
    let declared = server.raw(
        format!("POST /api/write HTTP/1.1\r\n{host}Content-Type: application/json\r\nContent-Length: 1048577\r\n\r\n")
            .as_bytes(),
    );
    declared.refused(413, "too-large");
    server
        .raw(format!("POST /api/write HTTP/1.1\r\n{host}Content-Length: 99999999999999999999999999\r\n\r\n").as_bytes())
        .refused(413, "too-large");
    // And one that is really sent.
    let big = json!({"op": "comment", "note": "x".repeat(1024 * 1024)});
    server.write(&big).refused(413, "too-large");
    // Just under the limit is read, and then refused or taken on its merits.
    let fits = json!({"op": "comment", "note": "x".repeat(900 * 1024)});
    assert_eq!(server.write(&fits).status, 200);
    std::fs::write(&file, &before).unwrap();

    // Headers larger than the server takes.
    let long = format!("GET / HTTP/1.1\r\n{host}X-Padding: {}\r\n\r\n", "a".repeat(20_000));
    server.raw(long.as_bytes()).refused(431, "too-large");
    let many: String = (0..2000).map(|n| format!("X-{n}: {n}\r\n")).collect();
    server
        .raw(format!("GET / HTTP/1.1\r\n{host}{many}\r\n").as_bytes())
        .refused(431, "too-large");

    // Things that are not requests.
    for garbage in [
        &b"\r\n\r\n"[..],
        b"GET\r\n\r\n",
        b"GET / HTTP/2.0\r\nHost: x\r\n\r\n",
        b"GET  / HTTP/1.1\r\nHost: x\r\n\r\n",
        b"GET / HTTP/1.1 extra\r\nHost: x\r\n\r\n",
        b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03\r\n\r\n",
        b"GET / HTTP/1.1\r\nno colon here\r\n\r\n",
        b"GET / HTTP/1.1\r\nBad Name: x\r\n\r\n",
        b"GET / HTTP/1.1\r\n\xff\xfe: x\r\n\r\n",
    ] {
        server.raw(garbage).refused(400, "bad-request");
    }
    let post = |extra: &str, body: &str| {
        server.raw(
            format!(
                "POST /api/write HTTP/1.1\r\n{host}X-Endmark-Token: {token}\r\nOrigin: {origin}\r\n\
                 Content-Type: application/json\r\n{extra}\r\n{body}"
            )
            .as_bytes(),
        )
    };
    post("", "").refused(411, "no-length");
    post("Content-Length: 2\r\nContent-Length: 2\r\n", "{}").refused(400, "bad-request");
    post("Content-Length: -1\r\n", "").refused(400, "bad-request");
    post("Content-Length: 2 \r\nTransfer-Encoding: chunked\r\n", "{}").refused(400, "bad-request");
    post("Transfer-Encoding: chunked\r\n", "2\r\n{}\r\n0\r\n\r\n").refused(400, "bad-request");
    // A body that is not text.
    let mut binary = format!(
        "POST /api/write HTTP/1.1\r\n{host}X-Endmark-Token: {token}\r\nOrigin: {origin}\r\n\
         Content-Type: application/json\r\nContent-Length: 4\r\n\r\n"
    )
    .into_bytes();
    binary.extend_from_slice(&[0xff, 0xfe, b'{', b'}']);
    server.raw(&binary).refused(400, "bad-request");

    // A client that stops half way, and one that connects and says nothing,
    // do not keep the server from answering the next one.
    let mut half = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    half.write_all(b"POST /api/write HTTP/1.1\r\nHost: ").unwrap();
    let silent: Vec<TcpStream> = (0..40)
        .map(|_| TcpStream::connect(("127.0.0.1", server.port)).unwrap())
        .collect();
    drop(half);
    // More connections than the server serves at once are open and silent.
    // When they go, it answers again.
    drop(silent);
    let mut answered = false;
    for _ in 0..50 {
        let alive = TcpStream::connect(("127.0.0.1", server.port)).and_then(|mut stream| {
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            stream.write_all(format!("GET / HTTP/1.1\r\n{host}\r\n").as_bytes())?;
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes)?;
            Ok(bytes)
        });
        if alive.is_ok_and(|bytes| bytes.starts_with(b"HTTP/1.1 200 OK\r\n")) {
            answered = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(answered, "the server still answers");
    assert_eq!(server.api("/api/file")["ok"], true);
    assert_eq!(std::fs::read(&file).unwrap(), before, "none of it wrote anything");
}

#[test]
fn nothing_but_the_page_can_be_fetched() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    std::fs::write(dir.0.join("secret.txt"), "secret").unwrap();
    let server = Server::start(&file, &["--by", "adam"]);
    let token = server.token();
    let accept = [("Accept-Encoding", "gzip"), ("X-Endmark-Token", token.as_str())];

    for path in [
        "/../Cargo.toml",
        "/../../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/%2E%2E%2FCargo.toml",
        "/..%2fCargo.toml",
        "/..%5cCargo.toml",
        "/..\\Cargo.toml",
        "/.%2e/%2e%2e/etc/passwd",
        "//etc/passwd",
        "/etc/passwd",
        "/secret.txt",
        "/notes.md",
        "/./notes.md",
        "/assets.rs",
        "/view/assets.rs",
        "/index.html/",
        "/index.html/../secret.txt",
        "/endmark.wasm.gz",
        "/endmark.wasm/",
        "/endmark.wasm%00",
        "/api",
        "/api/",
        "/api/file/",
        "/api/file/../../secret.txt",
        "/api/files",
        "/favicon.ico",
        "*",
        "secret.txt",
        "http://127.0.0.1/secret.txt",
        "file:///etc/passwd",
    ] {
        let reply = server.get(path, &accept);
        reply.refused(404, "not-found");
        assert!(!reply.text().contains("secret"), "{path}");
    }
    // The file being served has a name, and it does not help to ask by it.
    let named = format!("/{}", file.to_string_lossy().trim_start_matches('/'));
    server.get(&named, &accept).refused(404, "not-found");
    // A query does not change what a path names.
    assert_eq!(server.get("/?x=/../secret.txt", &accept).status, 200);
    assert_eq!(server.get("/endmark.wasm?v=1", &accept).status, 200);
}

#[cfg(unix)]
#[test]
fn a_hostile_file_name_stays_text() {
    let dir = TempDir::new();
    let name = "<img src=x onerror=alert(1)>\u{1b}[2J\"'.md";
    let file = dir.example(name);
    let server = Server::start(&file, &["--by", "adam"]);
    // The line for the terminal has no control character in it.
    assert!(!server.banner.contains('\u{1b}'), "{:?}", server.banner);
    assert!(server.banner.contains("<img src=x onerror=alert(1)>?[2J"));
    // The page does not contain the name. It gets it as JSON and sets it as text.
    assert!(!server.get("/", &[]).text().contains("onerror"));
    assert_eq!(server.api("/api/file")["file"], file.to_string_lossy().as_ref());
}

#[test]
fn it_refuses_to_start_on_wrong_arguments() {
    let dir = TempDir::new();
    let file = dir.example("notes.md");
    let file = file.to_string_lossy().into_owned();
    let missing = dir.0.join("missing.md").to_string_lossy().into_owned();
    let folder = dir.0.to_string_lossy().into_owned();
    let binary = dir.0.join("binary.md");
    std::fs::write(&binary, [0xff, 0xfe]).unwrap();
    let binary = binary.to_string_lossy().into_owned();
    // A port that is taken.
    let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = taken.local_addr().unwrap().port().to_string();

    for (args, says) in [
        (vec!["view", "--by", "adam"], "view needs a file"),
        (vec!["view", "--by", "adam", &file, &file], "view takes one file"),
        (vec!["view", "--by", "adam", "-"], "not standard input"),
        (vec!["view", &file], "view needs to know who is writing"),
        (vec!["view", "--by", "", &file], "view needs to know who is writing"),
        (vec!["view", "--by", "adam", &missing], "missing.md"),
        (vec!["view", "--read-only", &missing], "missing.md"),
        (vec!["view", "--by", "adam", &folder], &folder),
        (vec!["view", "--by", "adam", &binary], "not UTF-8"),
        (vec!["view", "--by", "adam", "--port", &port, &file], "is in use"),
        (
            vec!["view", "--by", "adam", "--port", "70000", &file],
            "--port takes a number",
        ),
        (
            vec!["view", "--by", "adam", "--port", "http", &file],
            "--port takes a number",
        ),
        (vec!["view", "--by", "adam", "--port"], "--port needs a value"),
        (
            vec!["view", "--by", "adam", "--host", "0.0.0.0", &file],
            "unknown option",
        ),
        (vec!["view", "--by", "adam", "--bind=0.0.0.0", &file], "unknown option"),
        (vec!["view", "--by", "adam", "--open", &file], "unknown option"),
    ] {
        let out = endmark(&args);
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(stderr.starts_with("endmark: "), "{args:?}: {stderr}");
        assert!(stderr.contains(says), "{args:?}: {stderr}");
        assert!(out.stdout.is_empty(), "{args:?}: it did not claim to serve");
    }
}

#[test]
fn it_listens_on_this_computer_only() {
    let dir = TempDir::new();
    let server = Server::start(&dir.example("notes.md"), &["--read-only"]);
    // The socket is bound to 127.0.0.1. Whatever other address this computer
    // has, nothing listens there on that port.
    let probe = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    // No packet is sent: connecting a UDP socket only picks the local address for a route.
    if probe.connect("192.0.2.1:9").is_ok() {
        if let Ok(local) = probe.local_addr() {
            if !local.ip().is_loopback() && !local.ip().is_unspecified() {
                let reached = TcpStream::connect_timeout(&(local.ip(), server.port).into(), Duration::from_secs(2));
                assert!(reached.is_err(), "the server answers on {}", local.ip());
            }
        }
    }
    assert_eq!(server.get("/", &[]).status, 200);
}
