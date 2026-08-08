//! Makes Cargo rebuild this crate when the files it embeds from outside the
//! package change.
//!
//! `tests/dom_parity.rs` uses the runtime's DOM shim, which `zdc-runtime`
//! owns and re-exports, and pulls the examples in with `include_str!` —
//! both of which live outside this package. Cargo fingerprints the package directory, so without these rules
//! the shim could change and the parity test would keep running the copy it
//! embedded last — a stale pass, which looks like success.

fn main() {
    println!("cargo:rerun-if-changed=../zdc-runtime/runtime/dom-shim.js");
    println!("cargo:rerun-if-changed=../../examples");
    println!("cargo:rerun-if-changed=../zdc-runtime/runtime");
}
