#![forbid(unsafe_code)]

//! The compiler as a filter: a program on standard input, JSON on
//! standard output.
//!
//! # Why an interface this plain
//!
//! This binary is what the browser runs. Compiled for `wasm32-wasip1` it
//! is a WebAssembly module whose only entry point is `_start`, which the
//! standard library provides — so **this crate exports no symbol of its
//! own, and that is the point**.
//!
//! The obvious design was three exported functions marshalling a string
//! through linear memory. It cannot be built here. `#[no_mangle]` and
//! `#[export_name]` are both refused by `#![forbid(unsafe_code)]` —
//! rustc's `unsafe_code` lint covers them, because overriding a symbol
//! name is exactly the kind of thing that makes linking unsound — and
//! every crate in this workspace carries that attribute.
//!
//! `wasm-bindgen` does compile under `forbid(unsafe_code)`, and that is
//! not a reason to reach for it. It works because lints do not fire on
//! code expanded from another crate's proc macro: the `unsafe` is still
//! generated into this crate, the compiler just cannot see it to complain.
//! Using it would mean this crate's `#![forbid(unsafe_code)]` was
//! decorative, in a workspace whose gates exist to make that attribute
//! mean something. It would also add twelve packages and a version-locked
//! `wasm-bindgen-cli` post-processing step, to a project whose pitch is
//! that you install one binary and nothing else.
//!
//! Standard input and standard output need none of that. They are the
//! oldest interface there is, `std` implements both for `wasm32-wasip1`,
//! and the host side is a WASI shim small enough to read in one sitting
//! (`playground/wasi.js`).
//!
//! # Why not `wasm32-unknown-unknown`, which is the usual answer
//!
//! Because a module for that target cannot be *given* anything. Both
//! builds work and their interfaces are worth putting side by side, read
//! straight out of the two `.wasm` files:
//!
//! ```text
//! wasm32-unknown-unknown   exports: main, memory   imports: (none)
//! wasm32-wasip1            exports: _start, memory imports: 13 WASI calls
//! ```
//!
//! No imports at all is not the achievement it looks like. It means there
//! is no call the module can make to reach its host, and with no exported
//! function of its own — see above for why there cannot be one — there is
//! no way for the host to reach it either, beyond calling `main` on an
//! empty world and receiving nothing back. `std`'s standard streams for
//! that target are stubs; there is nothing under them to write to.
//!
//! The library half of this crate still builds for
//! `wasm32-unknown-unknown`, and CI builds it there on purpose. It is the
//! stricter proof: a target with no host interface cannot be reaching a
//! syscall by any route, so the day a dependency starts opening files,
//! that build is where it is caught.
//!
//! # The cost, stated
//!
//! A WASI module runs `_start` once. The browser therefore instantiates a
//! fresh instance per compile, which costs an instantiation — not a
//! recompile, because `WebAssembly.Module` is compiled once and reused.
//! It also means one compile cannot leak state into the next, which for a
//! playground people will paste anything into is worth having.

use std::io::{Read, Write};

fn main() {
    let mut source = String::new();
    // Lossy on purpose, and it cannot happen from the playground: a
    // `<textarea>` produces UTF-8. Reading to a `String` with `?` would
    // turn a byte the host mangled into no output at all, and this
    // program's contract is that it always answers.
    let mut bytes = Vec::new();
    let read = std::io::stdin().read_to_end(&mut bytes);
    if read.is_ok() {
        source = String::from_utf8_lossy(&bytes).into_owned();
    }

    let json = zdc_wasm::compile_to_json(&source);

    // `write_all` rather than `println!`: the answer is one document and
    // a panic on a closed pipe would be a worse failure than a short
    // write. The host reads to end of stream, so no trailing newline is
    // needed and none is written — a stray byte would be inside nothing
    // and outside the JSON.
    let mut out = std::io::stdout();
    if out.write_all(json.as_bytes()).is_ok() {
        let _ = out.flush();
    }
}
