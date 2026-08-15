//! One number, spelled in more than one language.
//!
//! The wire format's version has to be known in three places that cannot
//! share a definition: `runtime/wire.js`, which the browser runs;
//! `zdc-deploy`'s `js/router.js`, which every deployed server runs and
//! which imports nothing; and Rust, where `zdc-dev` refuses a mismatched
//! request before any JavaScript is started. The repository's usual answer
//! — one definition, three users — is not available.
//!
//! What is available is the check. Each spelling is read back out of the
//! file that carries it and compared to [`zdc_runtime::WIRE_VERSION`], so
//! a version bumped in one place and not the others fails here rather than
//! in production, where the symptom would be every request refused for
//! naming the wrong format.
//!
//! `router.js` is read through the filesystem rather than through
//! `zdc-deploy`: this crate is upstream of that one, and depending on it
//! to test a constant would invert the dependency for one string.

use std::path::{Path, PathBuf};

use zdc_runtime::{WIRE_JS, WIRE_VERSION, WIRE_VERSION_HEADER, WIRE_VERSION_PARAM};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The integer a `const NAME = <n>;` declaration is given.
fn declared(source: &str, name: &str) -> Option<u32> {
    let at = source.find(name)?;
    let rest = &source[at + name.len()..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[test]
fn wire_js_and_rust_name_the_same_version() {
    assert_eq!(
        declared(WIRE_JS, "const VERSION"),
        Some(WIRE_VERSION),
        "`runtime/wire.js` and `zdc_runtime::WIRE_VERSION` name different \
         wire format versions. They are the two ends of every request, so \
         disagreeing here means every request is refused for naming a \
         format the other end does not speak."
    );
}

/// The two spellings of the header and the parameter, likewise.
///
/// A typo in either is not a refusal, it is a *silent* absence: a server
/// looking for `zd-wire` while the client sends `zdwire` sees no version
/// at all, and whether that is refused depends on how absence is treated.
/// It is treated as a mismatch — but a check that fires because of a typo
/// tells nobody what is actually wrong, so the spellings are pinned.
#[test]
fn the_header_and_parameter_are_spelled_the_same_on_both_sides() {
    for (rust, js) in [
        (WIRE_VERSION_HEADER, "VERSION_HEADER"),
        (WIRE_VERSION_PARAM, "VERSION_PARAM"),
    ] {
        let quoted = format!("const {js} = '{rust}'");
        assert!(
            WIRE_JS.contains(&quoted),
            "`runtime/wire.js` does not declare `{quoted}`, so the Rust and \
             JavaScript halves name the version in different places."
        );
    }
}

/// `zdc-deploy`'s router is the server half on every deployed target, and
/// it is copied verbatim rather than generated — so its copy of the number
/// is a third spelling and needs the same check.
#[test]
fn the_deployed_router_names_the_same_version() {
    let path = repository().join("crates/zdc-deploy/js/router.js");
    let router = std::fs::read_to_string(&path).expect("js/router.js is readable");
    assert_eq!(
        declared(&router, "const WIRE_VERSION"),
        Some(WIRE_VERSION),
        "`zdc-deploy/js/router.js` names a different wire format version \
         from `runtime/wire.js`. It is byte-identical on every target and \
         copied rather than generated, which is why it carries its own copy \
         of the number and why this test exists."
    );
    assert!(
        router.contains(&format!("const WIRE_HEADER = '{WIRE_VERSION_HEADER}'")),
        "`zdc-deploy/js/router.js` does not spell the header as \
         `{WIRE_VERSION_HEADER}`, so a deployed server would read the \
         version out of a header no client sends."
    );
    assert!(
        router.contains(&format!("const WIRE_PARAM = '{WIRE_VERSION_PARAM}'")),
        "`zdc-deploy/js/router.js` does not spell the subscription \
         parameter as `{WIRE_VERSION_PARAM}`."
    );
}
