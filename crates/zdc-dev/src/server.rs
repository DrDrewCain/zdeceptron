//! The HTTP server: static bundle, error page, and the live-reload stream.
//!
//! Publishing is separated from building on purpose. `DevServer` knows
//! nothing about files or watchers — it holds whatever `Site` it was last
//! given and tells connected browsers when that changed. A rebuild is
//! therefore a single call on `Handle`, which is what makes the reload
//! path testable without a filesystem, a timer, or a browser.

use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tiny_http::{Header, Request, Response, StatusCode};
use zdc_host::{Environment, Host};
use zdc_store::{DurableStore, EmbeddedStore, Keys};

use crate::compile::Site;
use crate::endpoints;
use crate::page;
use crate::sse::{self, Resume};

/// How long a stream may sit silent before a comment is written down it.
///
/// The comment is not for the browser, which is happy to wait: it is how
/// the server discovers that a tab was closed, since a socket nobody
/// writes to never reports that the peer is gone.
const HEARTBEAT: Duration = Duration::from_secs(15);

/// How long the stream loop waits before looking at the store again.
///
/// Short, because this is the latency of the two-window demo: the gap
/// between one window's click committing and the other window's frame
/// going out. It is a poll of an in-process channel, not of a database, so
/// the cost of a short interval is a wakeup on an idle thread.
///
/// A blocking select over both a channel and a subscription would remove
/// it entirely; `std::sync::mpsc` has no such select, and reaching for one
/// would mean an async runtime — a larger dependency than the server
/// hosting it, for a dev server serving one developer.
const POLL_GAP: Duration = Duration::from_millis(25);

/// What the server is currently serving, and how many times that has
/// changed. The two are one value under one lock because a client that
/// reads the generation and the site separately could read them from
/// either side of a rebuild.
struct Current {
    site: Arc<Site>,
    generation: u64,
}

struct Shared {
    current: Mutex<Current>,
    clients: Mutex<Vec<Sender<Vec<u8>>>>,
    /// The store behind `durable`, and the configuration behind `$env`.
    ///
    /// Outside `Current` on purpose: a rebuild replaces the program, not
    /// the data. A developer who edits a view and saves must not lose the
    /// count they just clicked up — that would make `durable` mean
    /// "survives a reload, but not a keystroke".
    store: Arc<dyn DurableStore>,
    env: Environment,
}

/// Publishes rebuilds to a running server. Cheap to clone, and safe to
/// use from a thread other than the one running `serve`.
#[derive(Clone)]
pub struct Handle {
    shared: Arc<Shared>,
}

impl Handle {
    /// Replace what is served and tell every connected browser to reload.
    ///
    /// A failed build is published exactly like a successful one: the
    /// developer should see the diagnostic appear without touching the
    /// browser, which is the whole point of putting it on the page.
    /// Returns the new generation.
    pub fn publish(&self, site: Site) -> u64 {
        let mut current = self
            .shared
            .current
            .lock()
            .expect("dev server state poisoned");
        current.site = Arc::new(site);
        current.generation += 1;
        let generation = current.generation;

        let frame = sse::frame(generation, sse::RELOAD, &generation.to_string()).into_bytes();
        let mut clients = self
            .shared
            .clients
            .lock()
            .expect("dev server clients poisoned");
        // A send fails only once the receiving stream has hung up, so this
        // is also how disconnected browsers are reaped.
        clients.retain(|client| client.send(frame.clone()).is_ok());
        generation
    }

    pub fn generation(&self) -> u64 {
        self.shared
            .current
            .lock()
            .expect("dev server state poisoned")
            .generation
    }

    /// How many browsers are currently subscribed to the reload stream.
    pub fn subscribers(&self) -> usize {
        self.shared
            .clients
            .lock()
            .expect("dev server clients poisoned")
            .len()
    }
}

pub struct DevServer {
    server: tiny_http::Server,
    shared: Arc<Shared>,
    addr: SocketAddr,
}

impl DevServer {
    /// Bind a port and start serving `site`.
    ///
    /// The listener is created here rather than inside `tiny_http` so that
    /// a port already in use surfaces as a real `io::Error` — the CLI has
    /// to tell "that port is taken" apart from every other failure, and a
    /// boxed error string cannot be matched on.
    pub fn bind(addr: SocketAddr, site: Site) -> io::Result<DevServer> {
        // In memory, not on disk. `zdc dev` never writes a `dist/`, and a
        // database file left in a project directory is one more thing a
        // developer has to know about and add to `.gitignore`. Durability
        // across a *rebuild* is what matters here and this provides it;
        // durability across a restart is `zdc-store`'s own test.
        let store: Arc<dyn DurableStore> =
            Arc::new(EmbeddedStore::in_memory().map_err(|e| io::Error::other(e.to_string()))?);
        DevServer::bind_with(addr, site, store, Environment::from_process())
    }

    /// Bind against a store and an environment the caller owns.
    ///
    /// What a test uses to look at the store directly, and what a future
    /// `--store` flag would call.
    pub fn bind_with(
        addr: SocketAddr,
        site: Site,
        store: Arc<dyn DurableStore>,
        env: Environment,
    ) -> io::Result<DevServer> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let server = tiny_http::Server::from_listener(listener, None)
            .map_err(|e| io::Error::other(e.to_string()))?;

        Ok(DevServer {
            server,
            shared: Arc::new(Shared {
                current: Mutex::new(Current {
                    site: Arc::new(site),
                    generation: 0,
                }),
                clients: Mutex::new(Vec::new()),
                store,
                env,
            }),
            addr,
        })
    }

    /// The address actually bound, which is what a caller that asked for
    /// port 0 needs in order to connect.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn handle(&self) -> Handle {
        Handle {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Serve until the listening socket closes.
    ///
    /// Ordinary requests are answered on this thread because every answer
    /// comes out of memory. Event streams get a thread each, since one
    /// stream occupies its connection for as long as the tab is open and
    /// answering it inline would stop the server dead.
    pub fn serve(&self) {
        for request in self.server.incoming_requests() {
            let path = crate::assets::normalize(request.url());
            if path == sse::LIVE_PATH || path == endpoints::LIVE || path == endpoints::WATCH {
                // A thread each: one stream occupies its connection for as
                // long as the tab is open, and answering it inline would
                // stop the server dead.
                let shared = Arc::clone(&self.shared);
                std::thread::spawn(move || stream(&shared, request));
            } else if path == endpoints::POLL {
                poll(&self.shared, request);
            } else if let Some(name) = endpoints::invocation(&path) {
                invoke(&self.shared, request, &name);
            } else {
                respond(&self.shared, request);
            }
        }
    }
}

/// Run one endpoint and answer with what it returned.
///
/// This is the request that used to 404. `zdc dev` served the emitted
/// function files as static assets, so `POST /_zd/greeting` looked for an
/// asset by that name, found none, and replied "not part of this bundle" —
/// which is how three generated server files came to be shipped without a
/// byte of them ever executing.
fn invoke(shared: &Shared, mut request: Request, name: &str) {
    let site = Arc::clone(
        &shared
            .current
            .lock()
            .expect("dev server state poisoned")
            .site,
    );
    let Site::Ready(ready) = &*site else {
        // The program does not compile, so there is no endpoint to run.
        // 503 and not 404: the endpoint may well exist in the source the
        // developer is halfway through fixing.
        return answer(
            request,
            503,
            "application/json; charset=utf-8",
            b"{\"error\":\"this program does not compile\"}".to_vec(),
        );
    };

    let mut body = String::new();
    if std::io::Read::read_to_string(request.as_reader(), &mut body).is_err() {
        return answer(
            request,
            400,
            "application/json; charset=utf-8",
            b"{\"error\":\"the request body is not UTF-8\"}".to_vec(),
        );
    }

    let host = Host::new(
        ready.endpoints.clone(),
        Arc::clone(&shared.store),
        shared.env.clone(),
    );
    match host.invoke(name, &body) {
        Ok(json) => answer(
            request,
            200,
            "application/json; charset=utf-8",
            json.into_bytes(),
        ),
        Err(error) => {
            // The message goes back in the body, because it is the text a
            // `Failed` variant renders in the browser: a developer looking
            // at a red bar on the page should see "`GREETING_API_KEY` is
            // not set" and not "500".
            let payload = format!("{{\"error\":{}}}", json_string(&error.to_string()));
            answer(
                request,
                error.status(),
                "application/json; charset=utf-8",
                payload.into_bytes(),
            )
        }
    }
}

/// One round of the polling transport.
///
/// The same events the stream carries, in an array, with the cursor in the
/// query string instead of a header. It exists because two common
/// deployment shapes — Lambda in buffered mode, and Lambda behind an ALB —
/// cannot hold a stream open at all, and a transport the dev server does
/// not implement is a transport nobody tests.
fn poll(shared: &Shared, request: Request) {
    let query = endpoints::Query::of(request.url());
    let Some(keys) = permitted(shared, &query) else {
        return answer(
            request,
            503,
            "application/json; charset=utf-8",
            b"{\"error\":\"this program does not compile\"}".to_vec(),
        );
    };

    let mut subscription = shared.store.watch(&keys, query.since());
    let mut events = Vec::new();
    // Drained without waiting. A long poll would be a better use of one
    // round trip and a worse fit for a blocking server with one thread per
    // held connection; the client's interval is the pacing.
    while let Some(event) = subscription.try_next() {
        events.push(endpoints::payload(&event));
    }
    answer(
        request,
        200,
        "application/json; charset=utf-8",
        format!("[{}]", events.join(",")).into_bytes(),
    )
}

/// The keys a subscriber asked for, narrowed to the ones this program
/// declares.
///
/// Narrowed rather than trusted: the query string comes from outside, and
/// a request for a key the program never declared would otherwise be a way
/// to read any value in the store by guessing its name.
fn permitted(shared: &Shared, query: &endpoints::Query) -> Option<Keys> {
    let site = Arc::clone(
        &shared
            .current
            .lock()
            .expect("dev server state poisoned")
            .site,
    );
    let Site::Ready(ready) = &*site else {
        return None;
    };
    let asked = query.keys();
    Some(Keys::new(
        ready
            .keys
            .iter()
            .filter(|key| asked.iter().any(|want| want == key))
            .map(str::to_string),
    ))
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if c == '"' {
            out.push_str("\\\"");
        } else if c == '\\' {
            out.push_str("\\\\");
        } else if c == '\n' {
            out.push_str("\\n");
        } else if c == '\r' {
            out.push_str("\\r");
        } else if c == '\t' {
            out.push_str("\\t");
        } else if (c as u32) < 0x20 {
            out.push_str(&format!("\\u{:04x}", c as u32));
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

fn answer(request: Request, status: u16, content_type: &str, body: Vec<u8>) {
    let response = Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"));
    let _ = request.respond(response);
}

/// Answer one request for a bundle file.
fn respond(shared: &Shared, request: Request) {
    let site = Arc::clone(
        &shared
            .current
            .lock()
            .expect("dev server state poisoned")
            .site,
    );
    let target = request.url().to_string();

    let (status, content_type, body): (u16, &str, Vec<u8>) = match &*site {
        Site::Ready(ready) => match ready.assets.get(&target) {
            Some(asset) => (200, asset.content_type, asset.body.clone()),
            None => (
                404,
                "text/plain; charset=utf-8",
                not_found(&ready.assets, &target).into_bytes(),
            ),
        },
        Site::Broken {
            source_path,
            report,
        } => {
            if is_document(&target) {
                // 200, not 5xx: the dev server did its job. The diagnostic
                // *is* the deliverable here (spec §7.3), and an error
                // status invites a browser or proxy to substitute its own
                // page for the one carrying it.
                (
                    200,
                    "text/html; charset=utf-8",
                    page::error_page(source_path, report).into_bytes(),
                )
            } else {
                // A stale page asking for `/client.js` must fail loudly.
                // Handing it HTML with a 200 would be a module parse error
                // in the console instead of a plain statement of the fact.
                (
                    503,
                    "text/plain; charset=utf-8",
                    format!("{target} is not available: this program does not compile.\n")
                        .into_bytes(),
                )
            }
        }
    };

    let response = Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"));
    let _ = request.respond(response);
}

/// Whether a target is the page itself rather than something it links to.
fn is_document(target: &str) -> bool {
    crate::assets::normalize(target) == "/index.html"
}

/// A 404 that names what *is* served.
///
/// The alternative — a bare "not found" — makes a mistyped path look
/// identical to a compiler that failed to emit the file.
fn not_found(assets: &crate::assets::Assets, target: &str) -> String {
    let mut out = format!("{target} is not part of this bundle.\n\nThis build serves:\n");
    for path in assets.paths() {
        out.push_str("  ");
        out.push_str(path);
        out.push('\n');
    }
    out
}

/// Hold one event stream open for as long as the browser keeps it.
///
/// **Two streams, one connection.** The live-reload channel and the
/// durable-sync channel are the same `text/event-stream`, distinguished by
/// the `event:` name, because a browser that opened one `EventSource` per
/// concern would spend two of its six connections per origin on a page
/// that has one thing to say.
///
/// The consequence is that the two share a cursor space, and they must
/// not: a rebuild generation and a store sequence number are different
/// numbers that both want to be the `Last-Event-ID`. The reload channel
/// keeps the id — it is the one whose resume decision is destructive, and
/// a missed reload leaves a stale page on screen — and every durable event
/// carries its own sequence in its payload, which is where
/// `runtime/store.js` reads it from. See the `seq` field in
/// `endpoints::payload`.
fn stream(shared: &Shared, request: Request) {
    let last_event_id = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Last-Event-ID"))
        .map(|h| h.value.as_str().to_string());
    let query = endpoints::Query::of(request.url());

    // Registered before the generation is read, so a rebuild that lands
    // between the two produces a redundant reload rather than a lost one.
    let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();
    let generation = {
        let current = shared.current.lock().expect("dev server state poisoned");
        shared
            .clients
            .lock()
            .expect("dev server clients poisoned")
            .push(tx);
        current.generation
    };

    let mut writer = request.into_writer();
    let mut opening = String::new();
    opening.push_str("HTTP/1.1 200 OK\r\n");
    opening.push_str("Content-Type: text/event-stream\r\n");
    opening.push_str("Cache-Control: no-store\r\n");
    opening.push_str("Connection: close\r\n");
    opening.push_str("\r\n");
    opening.push_str(&sse::preamble());

    let id = generation.to_string();
    opening.push_str(&match sse::resume(last_event_id.as_deref(), generation) {
        // The browser was away while the source changed — most often
        // because the server itself was restarted under an open tab.
        Resume::Missed => sse::frame(generation, sse::RELOAD, &id),
        Resume::UpToDate => sse::frame(generation, sse::READY, &id),
    });

    // The durable half of the stream, if this client asked for any keys.
    //
    // `since` comes from the query string rather than `Last-Event-ID`,
    // because that header is the reload generation — see the note above.
    // A client reconnecting therefore sends the cursor it tracked itself,
    // which is what `runtime/store.js` keeps.
    let mut durable = permitted(shared, &query)
        .filter(|keys| !keys.is_empty())
        .map(|keys| shared.store.watch(&keys, query.since()));

    if writer.write_all(opening.as_bytes()).is_err() || writer.flush().is_err() {
        return;
    }

    loop {
        // The store is drained first and without waiting: a durable update
        // is the one frame a second window is waiting on, and making it
        // queue behind a heartbeat timeout would put a visible delay on
        // the demo the whole placement exists for.
        let mut wrote = false;
        if let Some(subscription) = durable.as_mut() {
            while let Some(event) = subscription.try_next() {
                let frame = sse::frame(
                    endpoints::position(&event).0,
                    endpoints::UPDATE,
                    &endpoints::payload(&event),
                );
                if writer.write_all(frame.as_bytes()).is_err() {
                    return;
                }
                wrote = true;
            }
        }
        if wrote && writer.flush().is_err() {
            return;
        }

        // Then the reload channel, with a short wait rather than the full
        // heartbeat, so a durable write lands in milliseconds rather than
        // whenever this loop next comes round.
        let wait = if durable.is_some() {
            POLL_GAP
        } else {
            HEARTBEAT
        };
        let chunk = match rx.recv_timeout(wait) {
            Ok(chunk) => chunk,
            Err(RecvTimeoutError::Timeout) => {
                if durable.is_some() {
                    // Still connected, nothing to say, and the heartbeat is
                    // not due: go back and look at the store again.
                    continue;
                }
                sse::comment("keep-alive").into_bytes()
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if writer.write_all(&chunk).is_err() || writer.flush().is_err() {
            return;
        }
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .expect("the header names and values here are compile-time constants")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_stream_path_is_recognised_with_or_without_a_query() {
        assert_eq!(crate::assets::normalize(sse::LIVE_PATH), sse::LIVE_PATH);
        assert_eq!(
            crate::assets::normalize("/__zdc/live?since=3"),
            sse::LIVE_PATH
        );
    }

    #[test]
    fn only_the_page_itself_counts_as_a_document() {
        assert!(is_document("/"));
        assert!(is_document("/index.html"));
        assert!(!is_document("/client.js"));
        assert!(!is_document("/runtime/dom.js"));
    }

    #[test]
    fn a_404_names_what_the_bundle_does_contain() {
        let mut assets = crate::assets::Assets::default();
        assets.insert("/client.js", "export {}");
        let body = not_found(&assets, "/clietn.js");
        assert!(body.contains("/clietn.js"), "no target named:\n{body}");
        assert!(body.contains("/client.js"), "no inventory:\n{body}");
    }
}
