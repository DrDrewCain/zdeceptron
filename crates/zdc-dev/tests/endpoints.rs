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
    /// The response head, verbatim, so a test can ask which headers came
    /// back. The wire format's version is one of them (#144) and it is
    /// not visible in the body.
    head: String,
    body: String,
}

impl Reply {
    /// Whether the answer named this wire format version.
    fn names_wire(&self, version: &str) -> bool {
        self.head
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case(&format!("zd-wire: {version}")))
    }
}

/// A request written by hand, because what is being checked is that the
/// bytes a browser sends produce an answer — an HTTP client crate would be
/// a dependency added for the tests alone.
///
/// `wire` is the format version the caller names, `None` for a caller that
/// names none. Everything that stands in for a browser passes
/// `Some(WIRE_VERSION)`, because that is what `runtime/rpc.js` sends; the
/// other two spellings exist so the refusal can be tested deliberately.
fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
    wire: Option<&str>,
) -> Reply {
    let mut stream = TcpStream::connect(addr).expect("could not connect");
    stream.set_read_timeout(Some(TIMEOUT)).expect("timeout");

    let mut raw = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(body) = body {
        raw.push_str("Content-Type: application/json\r\n");
        raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    if let Some(wire) = wire {
        raw.push_str(&format!("{}: {wire}\r\n", zdc_runtime::WIRE_VERSION_HEADER));
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
        head: head.to_string(),
        body: body.to_string(),
    }
}

/// The wire format version a browser running this build would send.
fn ours() -> String {
    zdc_runtime::WIRE_VERSION.to_string()
}

fn post(addr: SocketAddr, path: &str, body: &str) -> Reply {
    request(addr, "POST", path, Some(body), Some(&ours()))
}

/// A POST naming some other wire format, or none.
fn post_claiming(addr: SocketAddr, path: &str, body: &str, wire: Option<&str>) -> Reply {
    request(addr, "POST", path, Some(body), wire)
}

fn get(addr: SocketAddr, path: &str) -> Reply {
    request(addr, "GET", path, None, Some(&ours()))
}

/// A GET that names no wire format — a static asset, or a caller from
/// before the format had a version.
fn get_unversioned(addr: SocketAddr, path: &str) -> Reply {
    request(addr, "GET", path, None, None)
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
fn an_unconfigured_secret_answers_with_a_reason_that_does_not_name_the_key() {
    // Inverted 2026-08-03. This asserted that the response body names
    // `GREETING_API_KEY`, on the argument that a developer staring at a
    // red bar should read the key name and not "500". That convenience
    // is what §16.3.12 assertion C forbids: the body is the text a
    // browser renders, `zdc dev` serves on whatever interface it was
    // given, and the key name tells an anonymous caller which credential
    // this deployment expects and so which service it talks to.
    //
    // It is still a 500 with a reason — just a reason that names no
    // configuration. The key name goes to the server's own console
    // through `HostError::detail`, which `Display` does not print.
    let running = start(site("guestbook.zd"), Environment::empty());
    let reply = post(running.addr, "/_zd/greeting", "[\"Ada\"]");
    assert_eq!(reply.status, 500);
    assert!(
        !reply.body.contains("GREETING_API_KEY"),
        "the response body names the environment key: {}",
        reply.body
    );
    assert!(
        reply.body.contains("environment"),
        "the failure says nothing a developer can act on: {}",
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
        let reply = get_unversioned(running.addr, path);
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
    let reply = get(running.addr, &format!("/_zd/poll?keys=visits&since=0&wire={}", ours()));
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
    let reply = get(running.addr, &format!("/_zd/poll?keys=visits&since=1&wire={}", ours()));
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
    let reply = get(running.addr, &format!("/_zd/poll?keys=private&since=0&wire={}", ours()));
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
    let reply = get_unversioned(running.addr, "/functions/visits.incr.js");
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

// --- the wire format's compatibility rule (#144) --------------------------

/// **The deliberate mismatch, over a real socket.**
///
/// This is the rolling deploy, reduced to its essentials: a client built
/// against one wire format posting to a server built against another. The
/// endpoint exists, the body is well-formed JSON, and the arguments are
/// the right arity — everything is right except the format the bytes are
/// written in.
///
/// Before the version existed the handler ran. `["Ada"]` decoded, the
/// greeting came back, and if the other format had spelled its values
/// differently the answer would have been wrong rather than absent, with
/// nothing anywhere saying which. The rule is that no compatibility is
/// promised, so this is a refusal — and the refusal names both numbers,
/// because "400" alone sends a developer looking at their arguments.
#[test]
fn a_post_naming_another_wire_format_is_refused_by_name() {
    let running = start(
        site("guestbook.zd"),
        Environment::from_pairs([("GREETING_API_KEY", "sk-test")]),
    );

    let reply = post_claiming(running.addr, "/_zd/greeting", "[\"Ada\"]", Some("2"));
    assert_eq!(
        reply.status, 400,
        "a mismatched wire format ran the handler anyway: {}",
        reply.body
    );
    assert!(
        reply.body.contains("wire format 2") && reply.body.contains(&format!("reads {}", ours())),
        "the refusal does not name both versions:\n{}",
        reply.body
    );
    assert!(
        !reply.body.contains("Hello"),
        "the handler ran despite the refusal:\n{}",
        reply.body
    );
}

/// Naming no version at all is the same refusal, and it is the case that
/// will actually happen.
///
/// A rollback to a build from before #144 sends no header. Treating that
/// silence as agreement would leave the rule open in precisely the
/// situation it was written for, so absence is a mismatch and says which
/// version was expected.
#[test]
fn a_post_naming_no_wire_format_is_refused_too() {
    let running = start(site("guestbook.zd"), Environment::empty());
    let reply = post_claiming(running.addr, "/_zd/visits.incr", "[1]", None);
    assert_eq!(reply.status, 400, "an unversioned POST ran: {}", reply.body);
    assert!(
        reply.body.contains("wire format none"),
        "the refusal does not say that no version was named:\n{}",
        reply.body
    );
    assert_eq!(
        running.store.get("visits").expect("get").is_none(),
        true,
        "the refused command still wrote to the store"
    );
}

/// The transaction endpoint is refused on the same rule.
///
/// It is the path that matters most: `~atomic` carries a handler's whole
/// write set, so a version mismatch read through would commit every one
/// of those writes from arguments decoded by the wrong rules — atomically,
/// which is the one thing that makes it worse rather than better.
#[test]
fn a_mismatched_transaction_is_refused_before_anything_commits() {
    let running = start(site("guestbook.zd"), Environment::empty());
    let reply = post_claiming(
        running.addr,
        "/_zd/~atomic",
        "[[\"visits.incr\",[1]]]",
        Some("999"),
    );
    assert_eq!(reply.status, 400, "a mismatched batch ran: {}", reply.body);
    assert!(
        running.store.get("visits").expect("get").is_none(),
        "a refused transaction still committed"
    );
}

/// The live-sync transports carry the version in the query, because
/// `EventSource` cannot set a header — and they are refused on it.
#[test]
fn a_subscription_naming_another_wire_format_is_refused() {
    let running = start(site("guestbook.zd"), Environment::empty());
    post(running.addr, "/_zd/visits.incr", "[1]");

    for path in [
        "/_zd/poll?keys=visits&since=0&wire=2",
        // No parameter at all: a page from before the format had a version.
        "/_zd/poll?keys=visits&since=0",
    ] {
        let reply = get_unversioned(running.addr, path);
        assert_eq!(reply.status, 400, "`{path}` was served: {}", reply.body);
        assert!(
            !reply.body.contains("\"value\":1"),
            "`{path}` was refused and still carried the value:\n{}",
            reply.body
        );
    }
}

/// Every answer on the boundary names the format it is written in, so the
/// browser can refuse a server that is *older* than it is.
///
/// The server's own check cannot cover that direction: a build from before
/// #144 does not inspect the request and does not stamp the response, so
/// what protects a page against a rollback is the header's absence being
/// noticed at the other end. This asserts the half that is this server's
/// to provide.
#[test]
fn every_boundary_answer_names_the_wire_format_it_is_written_in() {
    let running = start(site("guestbook.zd"), Environment::empty());
    let answers = [
        post(running.addr, "/_zd/visits", "[]"),
        post(running.addr, "/_zd/visits.incr", "[1]"),
        // A refusal names it too, or a client could not tell a refusal
        // from a server that has no opinion.
        post_claiming(running.addr, "/_zd/visits", "[]", Some("2")),
        get(running.addr, &format!("/_zd/poll?keys=visits&since=0&wire={}", ours())),
    ];
    for (index, reply) in answers.iter().enumerate() {
        assert!(
            reply.names_wire(&ours()),
            "answer {index} does not name the wire format:\n{}",
            reply.head
        );
    }

    // And `zdc dev`'s own reload script does not, because it is not on the
    // boundary and carries no ZD value — claiming a wire version for it
    // would be a promise about bytes this format does not govern.
    let reload = get_unversioned(running.addr, "/__zdc/live.js");
    assert_eq!(reload.status, 200, "the reload script is not served");
    assert!(
        !reload.names_wire(&ours()),
        "the reload script claims a wire format version:\n{}",
        reload.head
    );
}
