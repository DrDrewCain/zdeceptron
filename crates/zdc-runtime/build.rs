//! Makes Cargo rebuild this crate when the shared runtime sources change.
//!
//! `src/lib.rs` and the tests pull the JavaScript in with
//! `include_str!("../runtime/…")`. Cargo fingerprints source files, not
//! embedded ones, so without this rule `signal.js` could change and
//! `cargo test` would keep running the previously embedded copy — a stale
//! pass, which is worse than a failure because it looks like success.
//!
//! These files used to live in a `runtime/` at the repository root, which
//! `include_str!` could reach and `cargo package` could not: a crate may
//! only embed files inside its own directory, so a published
//! `zdc-runtime` failed to compile with nine missing-file errors. They now
//! live in the crate that owns them.
//!
//! One directory rule covers both the shipped sources (`signal.js`,
//! `dom.js`, `elements.js`) and the JavaScript suites embedded by the
//! tests (`signal.test.js`, `dom.test.js`). Cargo scans a directory path
//! recursively for modifications, so files added there later are covered
//! without touching this file again.
//!
//! The path is relative to the package root, per Cargo's build-script
//! documentation.

fn main() {
    println!("cargo:rerun-if-changed=runtime");
}
