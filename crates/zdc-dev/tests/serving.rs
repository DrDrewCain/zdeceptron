//! The server, over a real socket.
//!
//! Every test binds port 0 and asks the OS which port it got, so the suite
//! can run in parallel, twice at once, or on a machine where 4321 is busy.
//! Nothing here waits on a timer to decide whether something happened:
//! rebuilds are published directly through `Handle`, so the only timeouts
//! are the ones that make a *failing* test fail instead of hanging.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use zdc_dev::{build_once, DevServer, Handle, Settings, Site};

/// Long enough that a loaded CI machine will not trip it, short enough that
/// a genuine hang is reported as a failure rather than a stuck job.
const TIMEOUT: Duration = Duration::from_secs(10);

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

fn site(name: &str) -> Site {
    build_once(&example(name), &Settings::default())
}

fn broken_site() -> Site {
    // Unique per call: these tests run in parallel, and two of them
    // writing and deleting one path would fail for reasons that have
    // nothing to do with the server.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "zdc-serving-{}-{}.zd",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, "view\n    Text \"a\" Text \"b\"\n")
        .expect("could not write the broken source");
    let site = build_once(&path, &Settings::default());
    let _ = std::fs::remove_file(&path);
    assert!(!site.is_ready(), "this fixture is supposed to be broken");
    site
}

/// A server on an ephemeral port, serving on its own thread.
struct Running {
    addr: SocketAddr,
    handle: Handle,
    _server: Arc<DevServer>,
}

fn start(site: Site) -> Running {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let server = Arc::new(DevServer::bind(addr, site).expect("could not bind an ephemeral port"));
    let addr = server.local_addr();
    let handle = server.handle();

    let serving = Arc::clone(&server);
    std::thread::spawn(move || serving.serve());

    Running {
        addr,
        handle,
        _server: server,
    }
}

struct Reply {
    status: u16,
    headers: String,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        let name = format!("{}:", name.to_ascii_lowercase());
        self.headers
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with(&name))
            .map(|line| line[name.len()..].trim())
    }
}

/// A GET, written by hand. A dev server is checked most convincingly by
/// the same bytes a browser would send, and an HTTP client crate would be
/// a dependency added for the tests alone.
fn get(addr: SocketAddr, path: &str) -> Reply {
    let mut stream = TcpStream::connect(addr).expect("could not connect");
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("could not send");

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .expect("could not read the reply");
    let raw = String::from_utf8_lossy(&raw).into_owned();

    let (head, body) = raw
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("no header terminator in:\n{raw}"));
    let (status_line, headers) = head.split_once("\r\n").unwrap_or((head, ""));
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status code in {status_line:?}"));

    Reply {
        status,
        headers: headers.to_string(),
        body: body.to_string(),
    }
}

/// An open event stream, read incrementally.
struct Stream {
    socket: TcpStream,
    seen: String,
}

impl Stream {
    fn open(addr: SocketAddr, last_event_id: Option<&str>) -> Stream {
        let mut socket = TcpStream::connect(addr).expect("could not connect");
        socket.set_read_timeout(Some(TIMEOUT)).unwrap();
        let mut request = format!(
            "GET {} HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\n",
            zdc_dev::sse::LIVE_PATH
        );
        if let Some(id) = last_event_id {
            request.push_str(&format!("Last-Event-ID: {id}\r\n"));
        }
        request.push_str("\r\n");
        socket
            .write_all(request.as_bytes())
            .expect("could not send");
        Stream {
            socket,
            seen: String::new(),
        }
    }

    /// Read until `needle` appears, or fail after `TIMEOUT`.
    fn wait_for(&mut self, needle: &str) -> &str {
        let deadline = Instant::now() + TIMEOUT;
        let mut buffer = [0u8; 1024];
        while !self.seen.contains(needle) {
            if Instant::now() > deadline {
                panic!(
                    "timed out waiting for {needle:?} in the stream:\n{}",
                    self.seen
                );
            }
            match self.socket.read(&mut buffer) {
                Ok(0) => panic!("the stream closed before {needle:?}:\n{}", self.seen),
                Ok(n) => self.seen.push_str(&String::from_utf8_lossy(&buffer[..n])),
                Err(e) => panic!("read failed waiting for {needle:?}: {e}\n{}", self.seen),
            }
        }
        &self.seen
    }
}

#[test]
fn the_page_is_served_with_the_live_reload_client_in_it() {
    let running = start(site("counter.zd"));
    let reply = get(running.addr, "/");

    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(
        reply.body.contains("<div id=\"app\">"),
        "no mount point:\n{}",
        reply.body
    );
    // The live client is a file (#146), so the page names it and the
    // server answers for it. Both, or reload breaks silently.
    assert!(
        reply.body.contains("/__zdc/live.js"),
        "no live reload:\n{}",
        reply.body
    );
    let client = get(running.addr, "/__zdc/live.js");
    assert_eq!(client.status, 200);
    assert!(
        client.body.contains("EventSource"),
        "the live client is not served:\n{}",
        client.body
    );
}

#[test]
fn the_bundle_is_served_from_memory_with_the_right_types() {
    let running = start(site("counter.zd"));

    for (path, content_type, needle) in [
        (
            "/client.js",
            "text/javascript; charset=utf-8",
            "export function main",
        ),
        ("/styles.css", "text/css; charset=utf-8", "zd-col"),
        (
            "/manifest.json",
            "application/json; charset=utf-8",
            "\"entry\"",
        ),
        (
            "/runtime/signal.js",
            "text/javascript; charset=utf-8",
            "signal",
        ),
        ("/runtime/dom.js", "text/javascript; charset=utf-8", "mount"),
    ] {
        let reply = get(running.addr, path);
        assert_eq!(reply.status, 200, "{path} was not served");
        assert_eq!(reply.header("content-type"), Some(content_type), "{path}");
        assert!(
            reply.body.contains(needle),
            "{path} looks wrong:\n{}",
            reply.body
        );
    }
}

#[test]
fn nothing_is_cached_so_a_reload_cannot_show_the_previous_build() {
    // The single most consequential header here: a cached `client.js`
    // makes live reload look broken in a way that is very hard to see.
    let running = start(site("counter.zd"));
    for path in ["/", "/client.js", "/styles.css"] {
        let reply = get(running.addr, path);
        assert_eq!(reply.header("cache-control"), Some("no-store"), "{path}");
    }
}

#[test]
fn an_unknown_path_is_a_404_that_says_what_is_served() {
    let running = start(site("counter.zd"));
    let reply = get(running.addr, "/nope.js");

    assert_eq!(reply.status, 404);
    assert!(
        reply.body.contains("/client.js"),
        "no inventory:\n{}",
        reply.body
    );
}

#[test]
fn a_program_that_does_not_compile_puts_the_diagnostic_on_the_page() {
    let running = start(broken_site());
    let reply = get(running.addr, "/");

    assert_eq!(
        reply.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(
        reply.body.contains("line break"),
        "the diagnostic is not on the page:\n{}",
        reply.body
    );
    assert!(
        !reply.body.contains('\u{1b}'),
        "raw terminal escapes reached the browser:\n{}",
        reply.body
    );
    assert!(
        reply.body.contains("/__zdc/live.js"),
        "the error page must reload itself when the fix lands:\n{}",
        reply.body
    );
    // And the client is still served while the program is broken, which
    // is exactly when it matters.
    let client = get(running.addr, "/__zdc/live.js");
    assert_eq!(client.status, 200);
    assert!(client.body.contains("EventSource"), "{}", client.body);
}

#[test]
fn a_stale_page_asking_for_a_module_of_a_broken_build_fails_loudly() {
    // Serving the error page as JavaScript would surface as a module parse
    // error in the console, which explains nothing.
    let running = start(broken_site());
    let reply = get(running.addr, "/client.js");

    assert_eq!(reply.status, 503);
    assert_eq!(
        reply.header("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert!(
        reply.body.contains("does not compile"),
        "unclear:\n{}",
        reply.body
    );
}

#[test]
fn the_event_stream_opens_with_a_retry_and_the_current_generation() {
    let running = start(site("counter.zd"));
    let mut stream = Stream::open(running.addr, None);
    let seen = stream.wait_for("event: ready");

    assert!(
        seen.contains("text/event-stream"),
        "wrong content type:\n{seen}"
    );
    assert!(
        seen.contains("retry: 500"),
        "no reconnection delay:\n{seen}"
    );
    assert!(seen.contains("id: 0"), "no event id:\n{seen}");
}

#[test]
fn publishing_a_rebuild_sends_a_reload_to_every_open_stream() {
    let running = start(site("counter.zd"));
    let mut one = Stream::open(running.addr, None);
    let mut two = Stream::open(running.addr, None);
    one.wait_for("event: ready");
    two.wait_for("event: ready");

    // Wait until both streams have registered, so the broadcast cannot
    // race the second subscription.
    let deadline = Instant::now() + TIMEOUT;
    while running.handle.subscribers() < 2 {
        assert!(Instant::now() < deadline, "streams never registered");
        std::thread::sleep(Duration::from_millis(5));
    }

    running.handle.publish(site("hello.zd"));

    assert!(one.wait_for("event: reload").contains("id: 1"));
    assert!(two.wait_for("event: reload").contains("id: 1"));
}

#[test]
fn a_reload_is_followed_by_the_new_bundle_not_the_old_one() {
    // The point of the whole exercise: the reload the browser is told to
    // do must fetch something different from what it already has.
    let running = start(site("counter.zd"));
    let before = get(running.addr, "/client.js").body;
    assert!(
        before.contains("derived("),
        "counter.zd should derive:\n{before}"
    );

    let mut stream = Stream::open(running.addr, None);
    stream.wait_for("event: ready");
    running.handle.publish(site("hello.zd"));
    stream.wait_for("event: reload");

    let after = get(running.addr, "/client.js").body;
    assert_ne!(before, after, "the served bundle did not change");
    assert!(
        !after.contains("derived("),
        "still serving the old build:\n{after}"
    );
}

#[test]
fn a_failed_rebuild_replaces_the_app_with_the_diagnostic() {
    let running = start(site("counter.zd"));
    let mut stream = Stream::open(running.addr, None);
    stream.wait_for("event: ready");

    running.handle.publish(broken_site());
    stream.wait_for("event: reload");

    let reply = get(running.addr, "/");
    assert!(
        reply.body.contains("line break"),
        "the page did not become the diagnostic:\n{}",
        reply.body
    );
}

#[test]
fn a_client_that_reconnects_behind_the_current_build_is_told_to_reload() {
    // Spec §8.1: reconnection resumes from `Last-Event-ID`. Here that
    // means a tab that was asleep across a rebuild refreshes on reconnect
    // instead of sitting on a stale page until the next edit.
    let running = start(site("counter.zd"));
    running.handle.publish(site("hello.zd"));
    assert_eq!(running.handle.generation(), 1);

    let mut stream = Stream::open(running.addr, Some("0"));
    let seen = stream.wait_for("\n\n");
    assert!(
        seen.contains("event: reload"),
        "no catch-up reload:\n{seen}"
    );
}

#[test]
fn a_client_that_reconnects_up_to_date_is_not_reloaded() {
    let running = start(site("counter.zd"));
    running.handle.publish(site("hello.zd"));

    let mut stream = Stream::open(running.addr, Some("1"));
    let seen = stream.wait_for("event: ready");
    assert!(
        !seen.contains("event: reload"),
        "reloaded needlessly:\n{seen}"
    );
}

#[test]
fn a_closed_tab_stops_counting_as_a_subscriber() {
    // A server that never reaps disconnected streams leaks a thread and a
    // channel per reload of a long dev session.
    let running = start(site("counter.zd"));
    {
        let mut stream = Stream::open(running.addr, None);
        stream.wait_for("event: ready");
        let deadline = Instant::now() + TIMEOUT;
        while running.handle.subscribers() < 1 {
            assert!(Instant::now() < deadline, "the stream never registered");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // The disconnect is noticed on the next write, which is what publish
    // does, so one publish is required to reap it.
    let deadline = Instant::now() + TIMEOUT;
    loop {
        running.handle.publish(site("counter.zd"));
        if running.handle.subscribers() == 0 {
            break;
        }
        assert!(Instant::now() < deadline, "a closed tab was never reaped");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn two_servers_can_run_at_once_on_different_ports() {
    let one = start(site("counter.zd"));
    let two = start(site("hello.zd"));
    assert_ne!(one.addr, two.addr);
    assert!(get(one.addr, "/client.js").body.contains("derived("));
    assert!(!get(two.addr, "/client.js").body.contains("derived("));
}

#[test]
fn a_port_already_in_use_is_reported_rather_than_silently_taken_over() {
    let running = start(site("counter.zd"));
    let Err(error) = DevServer::bind(running.addr, site("counter.zd")) else {
        panic!("binding an occupied port must fail");
    };
    assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
}

/// `zdc dev` answers a route's URL the way the deployed site will.
///
/// A dev server that served a different layout would be testing a site
/// nobody ships: `/writing/routing` is a document here for the same
/// reason it is a document in `dist/`, and the module it loads sits where
/// `zdc build` puts it.
#[test]
fn a_routed_program_serves_every_url_it_declares() {
    let running = start(site("site.zd"));

    for url in ["/", "/writing", "/writing/routing", "/writing/folding"] {
        let reply = get(running.addr, url);
        assert_eq!(reply.status, 200, "{url} was not served");
        assert_eq!(
            reply.header("content-type"),
            Some("text/html; charset=utf-8"),
            "{url} must be a document"
        );
    }

    let reply = get(running.addr, "/pages/writing-routing.js");
    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert!(
        reply.body.contains("titleOf('routing')"),
        "the served module must be the specialised one:\n{}",
        reply.body
    );

    let manifest = get(running.addr, "/routes.json");
    assert_eq!(manifest.status, 200);
    assert!(
        manifest.body.contains("\"notFound\":\"/404\""),
        "{}",
        manifest.body
    );

    drop(running.handle);
}

/// A URL nothing claims gets the page the *program* wrote — the `None`
/// arm of `when page`, which exhaustiveness already forced it to write —
/// with a 404 status. The server has no opinion of its own about it.
#[test]
fn an_unclaimed_url_gets_the_programs_own_not_found_page() {
    let running = start(site("site.zd"));

    let reply = get(running.addr, "/writing/nothing-here");
    assert_eq!(reply.status, 404);
    assert_eq!(
        reply.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(
        reply.body.contains("/pages/not-found.boot.js"),
        "the 404 must load the program's own not-found document:\n{}",
        reply.body
    );
    let boot = get(running.addr, "/pages/not-found.boot.js");
    assert_eq!(boot.status, 200);
    assert!(
        boot.body.contains("/pages/not-found.js"),
        "and that module must import the not-found page:\n{}",
        boot.body
    );

    // A missing *asset* is still the server's plain report: handing a
    // stale page HTML where it asked for a module would be a parse error
    // in the console instead of a statement of the fact.
    let asset = get(running.addr, "/pages/nope.js");
    assert_eq!(asset.status, 404);
    assert_eq!(
        asset.header("content-type"),
        Some("text/plain; charset=utf-8")
    );

    drop(running.handle);
}
