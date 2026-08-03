//! Makes Cargo rebuild this crate when anything it embeds from outside the
//! package changes.
//!
//! `src/lib.rs` pulls in the runtime sources, the DOM shim that lives with
//! the runtime's tests, and the examples whose bundles are sized. Cargo
//! fingerprints the package directory, so without these rules an edit to
//! `dom.js` would leave the benchmark measuring the copy it embedded last —
//! a stale pass, which is worse than a failure because it looks like one.

fn main() {
    println!("cargo:rerun-if-changed=../zdc-runtime/tests/dom-shim.js");
    println!("cargo:rerun-if-changed=../../runtime");
    println!("cargo:rerun-if-changed=../../examples");
    println!("cargo:rerun-if-changed=../../BENCHMARKS.md");
}
