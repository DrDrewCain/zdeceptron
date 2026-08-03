//! `zdc dev`, end to end, over a real socket.
//!
//! # What used to happen here
//!
//! `zdc build examples/guestbook.zd` emitted three server functions and
//! `zdc dev` served them as **static text**. So:
//!
//! ```text
//! POST /_zd/greeting  →  404  "/_zd/greeting is not part of this bundle."
//! ```
//!
//! The compiler exited 0, the dev server started, the page loaded, and
//! every request the page made failed. This file is the test that would
//! have caught it: the request the browser actually sends, answered by the
//! function the compiler actually emitted, against the store `durable`
//! actually writes to.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use zdc_dev::{build_once, DevServer, Settings, Site};
use zdc_host::Environment;
use zdc_store::{DurableStore, EmbeddedStore, Json};

const TIMEOUT: Duration = Duration::from_secs(10);

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

fn site(name: &str) -> Site {
    let site = build_once(&example(name), &Settings::default());
    assert!(site.is_ready(), "{name} does not compile");
    site
}

struct Running {
    addr: SocketAddr,
    store: Arc<dyn DurableStore>,
    _server: Arc<DevServer>,
}

fn start(site: Site, env: Environment) -> Running {
    let store: Arc<dyn DurableStore> =
        Arc::new(EmbeddedStore::in_memory().expect("an in-memory store opens"));
    let server = Arc::new(
        DevServer::bind_with(
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            site,
            Arc::clone(&store),
            env,
        )
        .expect("could not bind an ephemeral port"),
    );
    let addr = server.local_addr();
    let serving = Arc::clone(&server);
    std::thread::spawn(move || serving.serve());
    Running {
        addr,
        store,
        _server: server,
    }
}

struct Reply {
    status: u16,
    body: String,
}

/// A request written by hand, because what is being checked is that the
/// bytes a browser sends produce an answer — an HTTP client crate would be
/// a dependency added for the tests alone.
fn request(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> Reply {
    let mut stream = TcpStream::connect(addr).expect("could not connect");
    stream.set_read_timeout(Some(TIMEOUT)).expect("timeout");

    let mut raw = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(body) = body {
        raw.push_str("Content-Type: application/json\r\n");
        raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    raw.push_str("\r\n");
    if let Some(body) = body {
        raw.push_str(body);
    }
    stream.write_all(raw.as_bytes()).expect("could not send");

    let mut received = Vec::new();
    stream
        .read_to_end(&mut received)
        .expect("could not read the reply");
    let received = String::from_utf8_lossy(&received).into_owned();
    let (head, body) = received
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("no header terminator in:\n{received}"));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status code in:\n{head}"));

    Reply {
        status,
        body: body.to_string(),
    }
}

fn post(addr: SocketAddr, path: &str, body: &str) -> Reply {
    request(addr, "POST", path, Some(body))
}

fn get(addr: SocketAddr, path: &str) -> Reply {
    request(addr, "GET", path, None)
}

#[test]
fn posting_to_a_generated_endpoint_runs_it() {
    // The request that used to 404.
    let running = start(
        site("guestbook.zd"),
        Environment::from_pairs([("GREETING_API_KEY", "sk-test")]),
    );
    let reply = post(running.addr, "/_zd/greeting", "[\"Ada\"]");
    assert_eq!(
        reply.status, 200,
        "the endpoint did not run: {}",
        reply.body
    );
    assert_eq!(reply.body, "\"Hello, Ada.\"");
}

#[test]
fn a_command_endpoint_writes_to_the_store_the_server_holds() {
    let running = start(site("guestbook.zd"), Environment::empty());

    let reply = post(running.addr, "/_zd/visits.incr", "[1]");
    assert_eq!(reply.status, 200, "the command did not run: {}", reply.body);
    assert_eq!(reply.body, "1");

    assert_eq!(
        running
            .store
            .get("visits")
            .expect("get")
            .map(Json::into_string),
        Some("1".to_string()),
        "the write did not reach the store"
    );
}

#[test]
fn reading_a_durable_signal_returns_what_was_written() {
    let running = start(site("guestbook.zd"), Environment::empty());
    post(running.addr, "/_zd/visits.incr", "[1]");
    post(running.addr, "/_zd/visits.incr", "[1]");
    assert_eq!(post(running.addr, "/_zd/visits", "[]").body, "2");
}

#[test]
fn a_durable_signal_starts_at_its_declared_value() {
    let running = start(site("guestbook.zd"), Environment::empty());
    assert_eq!(
        post(running.addr, "/_zd/visits", "[]").body,
        "0",
        "an unwritten key rendered as something other than `starting 0`"
    );
}

#[test]
fn an_unconfigured_secret_answers_with_the_reason() {
    // 500 and a message naming the key, because that message is the text
    // the browser renders inside `Failed with error show ErrorBar` — a
    // developer staring at a red bar should read "`GREETING_API_KEY` is
    // not set", not "500".
    let running = start(site("guestbook.zd"), Environment::empty());
    let reply = post(running.addr, "/_zd/greeting", "[\"Ada\"]");
    assert_eq!(reply.status, 500);
    assert!(
        reply.body.contains("GREETING_API_KEY"),
        "the failure does not name the key: {}",
        reply.body
    );
}

#[test]
fn an_endpoint_this_build_does_not_have_is_a_404() {
    let running = start(site("guestbook.zd"), Environment::empty());
    assert_eq!(post(running.addr, "/_zd/visits.decr", "[1]").status, 404);
}

#[test]
fn a_malformed_body_is_the_callers_fault() {
    let running = start(site("guestbook.zd"), Environment::empty());
    assert_eq!(
        post(running.addr, "/_zd/visits.incr", "not json").status,
        400
    );
}

#[test]
fn the_secret_is_never_in_anything_the_browser_can_fetch() {
    // The value is configured and reachable by the handler. It must not be
    // in the page, the bundle, the manifest, or the emitted server file the
    // dev server also serves as text for reading.
    let running = start(
        site("guestbook.zd"),
        Environment::from_pairs([("GREETING_API_KEY", "sk-do-not-leak")]),
    );
    for path in [
        "/",
        "/client.js",
        "/manifest.json",
        "/runtime/store.js",
        "/functions/greeting.js",
    ] {
        let reply = get(running.addr, path);
        assert!(
            !reply.body.contains("sk-do-not-leak"),
            "{path} carries the secret:\n{}",
            reply.body
        );
    }
    // And it does reach the handler, so this is not passing by the secret
    // being unconfigured.
    assert_eq!(post(running.addr, "/_zd/greeting", "[\"Ada\"]").status, 200);
}

#[test]
fn polling_returns_the_writes_a_subscriber_missed() {
    // The fallback transport, over the wire. Two shapes cannot hold a
    // stream at all — Lambda buffered and Lambda behind an ALB — so this
    // is the path that has to work where the stream cannot.
    let running = start(site("guestbook.zd"), Environment::empty());

    // Subscribe from the beginning, then write.
    post(running.addr, "/_zd/visits.incr", "[1]");
    let reply = get(running.addr, "/_zd/poll?keys=visits&since=0");
    assert_eq!(reply.status, 200);
    assert!(
        reply.body.contains("\"key\":\"visits\"") && reply.body.contains("\"value\":1"),
        "the poll did not carry the write:\n{}",
        reply.body
    );
    assert!(
        reply.body.contains("\"seq\":1"),
        "the poll carried no cursor, so a reconnection could not resume:\n{}",
        reply.body
    );
}

#[test]
fn polling_from_the_current_position_returns_nothing() {
    let running = start(site("guestbook.zd"), Environment::empty());
    post(running.addr, "/_zd/visits.incr", "[1]");
    let reply = get(running.addr, "/_zd/poll?keys=visits&since=1");
    assert_eq!(reply.body, "[]", "an up-to-date poll replayed something");
}

#[test]
fn a_subscription_cannot_ask_for_a_key_the_program_never_declared() {
    // The query string comes from outside. Without narrowing it to the
    // declared key set, guessing a name would be a way to read any value
    // in the store.
    let running = start(site("guestbook.zd"), Environment::empty());
    running
        .store
        .set("private", Json::from_text("\"secret\""))
        .expect("set");
    let reply = get(running.addr, "/_zd/poll?keys=private&since=0");
    assert_eq!(
        reply.body, "[]",
        "an undeclared key was readable: {}",
        reply.body
    );
}

#[test]
fn the_generated_function_is_still_readable_as_text() {
    // Running them did not stop them being inspectable — §9's "see what
    // the split produced" is why they were served in the first place.
    let running = start(site("guestbook.zd"), Environment::empty());
    let reply = get(running.addr, "/functions/visits.incr.js");
    assert_eq!(reply.status, 200);
    assert!(
        reply.body.contains("$store.incr"),
        "the emitted source is not being served:\n{}",
        reply.body
    );
}

#[test]
fn an_endpoint_of_a_broken_program_says_so_rather_than_404() {
    // 503, not 404: the endpoint probably does exist in the source the
    // developer is halfway through fixing, and "no such endpoint" would
    // send them looking for a rename that never happened.
    let path = std::env::temp_dir().join(format!("zdc-endpoints-{}.zd", std::process::id()));
    std::fs::write(&path, "view Text\n").expect("could not write the fixture");
    let broken = build_once(&path, &Settings::default());
    let _ = std::fs::remove_file(&path);
    assert!(!broken.is_ready(), "this fixture is supposed to be broken");

    let running = start(broken, Environment::empty());
    assert_eq!(post(running.addr, "/_zd/greeting", "[\"Ada\"]").status, 503);
}
