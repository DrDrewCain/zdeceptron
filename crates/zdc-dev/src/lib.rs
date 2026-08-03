#![forbid(unsafe_code)]

//! `zdc dev` — build, watch, serve, reload.
//!
//! Spec §9 lists `zdc dev` first among the deployment commands, and it is
//! the first command anyone runs. Everything it needs is in this binary:
//! the compiler, the JavaScript runtime it serves, the HTTP server, and the
//! file watcher. There is no Node to install, no npm to run, and no
//! bundler to configure, because a language whose pitch is that you do not
//! think about infrastructure cannot open by asking you to install some
//! (§7).
//!
//! **All four placements, and they run.** `server` and `durable` are not
//! refused and never were; what is new is that the emitted handlers are
//! now executed rather than served as text. `zdc-host` binds `$env` and
//! `$store`, so `POST /_zd/<endpoint>` answers with a value, and
//! `/_zd/live` carries durable writes to every open window. `static` is
//! evaluated by the same build root `zdc build` runs, so the two cannot
//! disagree about what a program does.
//!
//! # Shape
//!
//! ```text
//! compile ──► Site ──► Handle::publish ──► SSE `reload` ──► browser
//!    ▲                                                          │
//!    └────────────── Watcher::changed ◄── save ◄────────────────┘
//! ```
//!
//! The four parts are separable and separately tested: [`compile`] is a
//! pure function of a path, [`server::Handle::publish`] needs no
//! filesystem, [`watch::Watcher`] needs no server, and [`sse`] needs
//! neither.

pub mod ansi;
pub mod assets;
pub mod compile;
pub mod endpoints;
pub mod page;
pub mod server;
pub mod sse;
pub mod watch;

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use zdc_diagnostics::{render, Diagnostic};

pub use crate::assets::{Asset, Assets};
pub use crate::compile::{compile, Ready, Settings, Site};
pub use crate::server::{DevServer, Handle};
pub use crate::watch::Watcher;

/// The port `zdc dev` listens on unless told otherwise.
///
/// High enough to need no privileges, and not one of the ports the popular
/// JavaScript toolchains have already claimed — a developer running one of
/// those beside `zdc dev` should not have to think about it.
pub const DEFAULT_PORT: u16 = 4321;

/// How to run the dev server.
#[derive(Debug, Clone)]
pub struct Options {
    /// The `.zd` file to serve.
    pub file: PathBuf,
    pub host: IpAddr,
    pub port: u16,
    /// How often the watch set is checked. See [`watch`] for why this is a
    /// poll and not a subscription.
    pub poll: Duration,
    pub settings: Settings,
}

impl Options {
    pub fn new(file: impl Into<PathBuf>) -> Options {
        Options {
            file: file.into(),
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: DEFAULT_PORT,
            poll: watch::POLL,
            settings: Settings::default(),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

/// A reason the server could not start at all.
///
/// Deliberately *not* a variant for "the program does not compile": that
/// is not a startup failure. The fix is one keystroke away and the server
/// keeps watching for it.
#[derive(Debug)]
pub enum StartupError {
    /// The entry file is not there, or cannot be read. A typo in an
    /// argument, not a mistake in a program — there is nothing to watch,
    /// so there is no reason to stay up.
    Unreadable { path: String, source: io::Error },
    /// The port is taken, or the address cannot be bound.
    Bind { addr: SocketAddr, source: io::Error },
}

impl StartupError {
    /// The message, rendered the way every other `zdc` failure is.
    pub fn report(&self) -> String {
        let (path, diagnostic) = match self {
            StartupError::Unreadable { path, source } => (
                path.clone(),
                Diagnostic {
                    message: format!("Could not read {path}: {source}"),
                    span: None,
                    notes: Vec::new(),
                    help: Some("`zdc dev` takes the path to a `.zd` file.".to_string()),
                    code: None,
                },
            ),
            StartupError::Bind { addr, source } => (
                addr.to_string(),
                Diagnostic {
                    message: format!("Could not listen on {addr}: {source}"),
                    span: None,
                    notes: Vec::new(),
                    help: Some(
                        "Another process is probably already using that port. Pass `--port` to \
                         choose a different one."
                            .to_string(),
                    ),
                    code: None,
                },
            ),
        };
        render("", &path, &diagnostic)
    }
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.report().trim_end())
    }
}

impl std::error::Error for StartupError {}

/// Build, serve, and keep serving until the process is stopped.
///
/// Returns only on a startup failure. A program that does not compile is
/// *not* one: the diagnostic goes to the terminal and to the page, and the
/// watcher keeps running, because the developer is about to fix it.
pub fn run(options: &Options) -> Result<(), StartupError> {
    // Checked before anything else so a mistyped path fails immediately
    // and unambiguously, rather than starting a server that will only ever
    // serve one diagnostic.
    if let Err(source) = std::fs::metadata(&options.file) {
        return Err(StartupError::Unreadable {
            path: options.file.display().to_string(),
            source,
        });
    }

    let watch_set = watch::watch_set(&options.file);
    let mut watcher = Watcher::new(watch_set);

    let site = compile(&options.file, &options.settings);
    announce(&site, None);

    let addr = options.addr();
    let server = Arc::new(
        DevServer::bind(addr, site).map_err(|source| StartupError::Bind { addr, source })?,
    );
    let handle = server.handle();

    println!(
        "zdc dev · http://{} · watching {}",
        server.local_addr(),
        options.file.display()
    );
    println!("Press Ctrl-C to stop.");

    let serving = Arc::clone(&server);
    std::thread::spawn(move || serving.serve());

    loop {
        std::thread::sleep(options.poll);
        if !watcher.changed() {
            continue;
        }
        let started = Instant::now();
        let site = compile(&options.file, &options.settings);
        announce(&site, Some(started.elapsed()));
        handle.publish(site);
    }
}

/// Report a build to the terminal.
///
/// A failure prints the diagnostics verbatim — the same bytes `zdc build`
/// would have printed — because the terminal and the page must not tell
/// two different stories about one error (spec §7.3).
fn announce(site: &Site, elapsed: Option<Duration>) {
    if let Some(report) = site.report() {
        eprint!("{report}");
        return;
    }
    // The first build says nothing: the banner that follows it is the
    // confirmation, and two lines for one event is noise.
    if let Some(elapsed) = elapsed {
        println!("rebuilt in {} ms", elapsed.as_millis());
    }
}

/// Convenience for callers that want one build without a server: the same
/// function `run` uses, so they cannot diverge.
pub fn build_once(file: &Path, settings: &Settings) -> Site {
    compile(file, settings)
}
