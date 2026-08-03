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

use crate::compile::Site;
use crate::page;
use crate::sse::{self, Resume};

/// How long a stream may sit silent before a comment is written down it.
///
/// The comment is not for the browser, which is happy to wait: it is how
/// the server discovers that a tab was closed, since a socket nobody
/// writes to never reports that the peer is gone.
const HEARTBEAT: Duration = Duration::from_secs(15);

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
            if crate::assets::normalize(request.url()) == sse::LIVE_PATH {
                let shared = Arc::clone(&self.shared);
                std::thread::spawn(move || stream(&shared, request));
            } else {
                respond(&self.shared, request);
            }
        }
    }
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
        Site::Ready(assets) => match assets.get(&target) {
            Some(asset) => (200, asset.content_type, asset.body.clone()),
            None => (
                404,
                "text/plain; charset=utf-8",
                not_found(assets, &target).into_bytes(),
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
fn stream(shared: &Shared, request: Request) {
    let last_event_id = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Last-Event-ID"))
        .map(|h| h.value.as_str().to_string());

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

    if writer.write_all(opening.as_bytes()).is_err() || writer.flush().is_err() {
        return;
    }

    loop {
        let chunk = match rx.recv_timeout(HEARTBEAT) {
            Ok(chunk) => chunk,
            Err(RecvTimeoutError::Timeout) => sse::comment("keep-alive").into_bytes(),
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
