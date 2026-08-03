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

pub mod ansi;
pub mod assets;
pub mod page;
pub mod sse;

pub use crate::assets::{Asset, Assets};
