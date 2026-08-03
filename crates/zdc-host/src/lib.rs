#![forbid(unsafe_code)]

//! The platform adapter (§8.2), for the one platform that has to work
//! first: this machine.
//!
//! # What was missing
//!
//! `zdc build` emits server files whose only free names are `$env` and
//! `$store`, above a comment saying both are "injected by the platform
//! adapter (§8.2)". No adapter existed. The compiler printed those files,
//! exited 0, and nothing had ever executed a byte of them — `zdc dev`
//! served them as static assets, so `POST /_zd/greeting` answered "not part
//! of this bundle". A green exit code hid all of it.
//!
//! This crate is the adapter. It binds the two names and runs the handler,
//! which is what turns "was printed" into "runs".
//!
//! # Why it interprets JavaScript rather than re-implementing it
//!
//! The obvious shortcut is to skip the emitted file: the host knows the
//! endpoint is `visits.incr`, so it could call `store.incr` itself. That
//! would test the *manifest* and leave the emitted bytes exactly as
//! unexecuted as they were. The emitted file is the deliverable, so the
//! emitted file is what runs here — through the same pure-Rust engine
//! `zdc-runtime` already uses for the client half, so verifying it still
//! installs nothing (§7).
//!
//! # What this is not
//!
//! Not a production runtime. It is single-threaded per invocation, builds
//! a fresh context per request, and has no cold-start story — because the
//! platforms that need one are Lambda, Workers, Deno Deploy, Vercel and
//! Azure, each of which registers a handler differently (ECMA-429
//! standardises the *interior* of a handler and no entrypoint at all; the
//! committee's `proposal-http-server-api` repository is empty). Those
//! adapters are shims over the same two bindings. This one is the
//! reference they have to agree with.

pub mod bindings;
pub mod endpoint;
pub mod env;

pub use crate::endpoint::{Endpoint, Endpoints, Shape};
pub use crate::env::Environment;

use std::sync::Arc;

use zdc_store::DurableStore;

/// A reason an invocation produced no answer.
///
/// Split by *who is wrong*, because the dev server turns these into status
/// codes and a caller cannot tell "you posted nonsense" from "the handler
/// threw" out of one string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// No endpoint by that name. The browser asked for something this
    /// build does not serve — most often a stale tab after a rename.
    Unknown { name: String },
    /// The request body was not the shape the endpoint's signature takes.
    BadRequest { message: String },
    /// The handler ran and threw, or failed to parse.
    ///
    /// `message` is **browser-visible**: it is the text a `Failed` variant
    /// renders, so it is bound by §16.3.12 assertion C and may name no
    /// `environment` key. The old doc here read "its own message, because
    /// that is the part naming which store key or which secret", and that
    /// was the leak written down as a feature.
    ///
    /// `detail` is the part that names it, and it never crosses the
    /// boundary: `Display` omits it, so an adapter that renders a
    /// `HostError` into a response body cannot emit it by accident. Only
    /// a caller that asks for it by name — `zdc dev`, writing its own
    /// console — can see it.
    Failed {
        endpoint: String,
        message: String,
        detail: Option<String>,
    },
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Unknown { name } => {
                write!(f, "`{name}` is not an endpoint in this build")
            }
            HostError::BadRequest { message } => f.write_str(message),
            // `detail` is deliberately absent. `Display` is what the dev
            // server and every deployed adapter put in the response body,
            // so anything printed here reaches a browser.
            HostError::Failed {
                endpoint,
                message,
                detail: _,
            } => {
                write!(f, "`{endpoint}` failed: {message}")
            }
        }
    }
}

impl std::error::Error for HostError {}

impl HostError {
    /// The HTTP status this maps to, decided here so the dev server and a
    /// deployed adapter cannot answer the same failure differently.
    pub fn status(&self) -> u16 {
        match self {
            HostError::Unknown { .. } => 404,
            HostError::BadRequest { .. } => 400,
            // Not 400: the request was well-formed and the server could
            // not honour it. A `Remote of T` renders either as `Failed`,
            // but a proxy and a log reader need the difference.
            HostError::Failed { .. } => 500,
        }
    }

    /// The server-side half of a failure: the text that names the
    /// `environment` key, the store key or the path.
    ///
    /// Asked for by name and never rendered, so a caller that writes a
    /// response body gets it only by deciding to.
    pub fn detail(&self) -> Option<&str> {
        match self {
            HostError::Unknown { .. } | HostError::BadRequest { .. } => None,
            HostError::Failed { detail, .. } => detail.as_deref(),
        }
    }
}

/// Everything a request needs: the endpoints, the store behind `durable`,
/// and the configuration behind `$env`.
pub struct Host {
    endpoints: Endpoints,
    store: Arc<dyn DurableStore>,
    env: Environment,
}

impl Host {
    pub fn new(endpoints: Endpoints, store: Arc<dyn DurableStore>, env: Environment) -> Host {
        Host {
            endpoints,
            store,
            env,
        }
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    pub fn store(&self) -> &Arc<dyn DurableStore> {
        &self.store
    }

    /// Run one endpoint against JSON arguments, returning JSON.
    ///
    /// `arguments_json` is the request body: the array the browser's
    /// `$remote` or `$call` sent. JSON in and JSON out, because that is
    /// what crosses the wire and re-encoding it into a richer type here
    /// would create a second definition of the contract.
    pub fn invoke(&self, name: &str, arguments_json: &str) -> Result<String, HostError> {
        let endpoint = self.endpoints.get(name).ok_or_else(|| HostError::Unknown {
            name: name.to_string(),
        })?;
        bindings::run(endpoint, &self.store, &self.env, arguments_json)
    }
}
