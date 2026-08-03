//! Makes Cargo rebuild this crate when the shared runtime sources change.
//!
//! `src/lib.rs` and `tests/render.rs` pull the JavaScript in with
//! `include_str!("../../../runtime/…")`, which resolves outside this
//! package. Cargo fingerprints the package directory, so edits to those
//! files did not invalidate the build: `signal.js` could change and
//! `cargo test` would keep running the previously embedded copy — a stale
//! pass, which is worse than a failure because it looks like success.
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
    println!("cargo:rerun-if-changed=../../runtime");
}
