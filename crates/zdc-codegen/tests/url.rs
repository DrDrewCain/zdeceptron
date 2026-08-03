//! The halves of the URL rule, run against one table.
//!
//! `zdc_hir::url_is_safe` settles every URL the compiler can see — a
//! written `Link` destination, and a markdown link in a post read at build
//! time. `safeUrl` in `runtime/dom.js` settles the ones it cannot, because
//! they are not known until the program runs. Two allowlists that can
//! drift are one allowlist too many: a scheme added to the Rust side and
//! not the JavaScript side is a URL the compiler admits and the runtime
//! blanks, and a scheme added to the JavaScript side and not the Rust side
//! is a hole with a test suite that says otherwise.
//!
//! So the table below is the only place either rule is stated in a test,
//! and both halves answer it — the JavaScript half inside a real engine,
//! running the exact bytes that ship.

mod support;

use support::context;

use boa_engine::{Context, Source};

/// One URL, and whether both halves must accept it.
///
/// The verdict is written out rather than computed from either
/// implementation: a table that asked one side what it thought would agree
/// with itself and could never fail.
const CASES: &[(&str, bool)] = &[
    // Relative, which is the commonest URL in a program and always allowed.
    ("", true),
    ("/", true),
    ("/notes/signals", true),
    ("notes.html", true),
    ("../up", true),
    ("#anchor", true),
    ("?q=1", true),
    // A colon after a `/`, `?` or `#` is inside a path or a query, not a
    // scheme. `/a:b` is relative, and refusing it would refuse a file whose
    // name contains a colon.
    ("/a:b", true),
    ("notes/a:b.md", true),
    ("?q=a:b", true),
    ("#a:b", true),
    ("https://example.com/a:b", true),
    // The allowlist, and its case-insensitivity: schemes are compared
    // ASCII-case-insensitively because the URL parser does.
    ("http://example.com", true),
    ("https://example.com/feed.xml", true),
    ("HTTPS://example.com", true),
    ("HtTpS://example.com", true),
    ("mailto:someone@example.com", true),
    // The two entries the two sets disagreed about, settled. `tel` is in,
    // because it hands the value to the dialer and opens no document;
    // `ftp` is out, because no shipping browser has resolved one since
    // 2021 and it was in the markdown half by convention rather than by
    // decision.
    ("tel:+441234567890", true),
    ("TEL:+441234567890", true),
    ("ftp://example.com/f", false),
    ("FTP://example.com/f", false),
    // The three schemes usually named, refused — and refused however they
    // are spelled, because the browser does not care about case either.
    ("javascript:alert(1)", false),
    ("JavaScript:alert(1)", false),
    ("JAVASCRIPT:alert(1)", false),
    ("vbscript:msgbox(1)", false),
    ("data:text/html,<script>alert(1)</script>", false),
    // Leading whitespace is stripped before the scheme is parsed, by the
    // browser and therefore by both halves. This is the bypass an allowlist
    // that trimmed nothing would have.
    (" javascript:alert(1)", false),
    ("\njavascript:alert(1)", false),
    ("\t\r\n javascript:alert(1)", false),
    // Not on the allowlist, and not dangerous in any obvious way — which is
    // the point of an allowlist. `file:` and `blob:` are refused for the
    // same reason a scheme invented tomorrow will be.
    ("file:///etc/passwd", false),
    ("blob:https://example.com/abc", false),
    ("ws://example.com", false),
    ("unknown-scheme:whatever", false),
];

/// `url` as a JavaScript single-quoted string literal.
fn js_string(url: &str) -> String {
    let mut out = String::from("'");
    for ch in url.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

/// What `safeUrl` did to `url`: it gives the URL back unchanged when it is
/// safe and the empty string when it is not.
///
/// Compared against the input rather than against `''`, which is what makes
/// the empty URL — safe, and unchanged — read as acceptance instead of as a
/// refusal.
fn safe_url_accepts(context: &mut Context, url: &str) -> bool {
    let literal = js_string(url);
    let script = format!("safeUrl({literal}) === {literal} ? 'yes' : 'no'");
    let verdict = context
        .eval(Source::from_bytes(script.as_bytes()))
        .unwrap_or_else(|e| panic!("safeUrl({literal}) failed: {e}"))
        .to_string(context)
        .expect("a string")
        .to_std_string_escaped();
    match verdict.as_str() {
        "yes" => true,
        "no" => false,
        other => panic!("safeUrl({literal}) gave {other}"),
    }
}

#[test]
fn both_halves_of_the_url_rule_agree_on_every_case() {
    let mut context = context(false);
    for &(url, safe) in CASES {
        assert_eq!(
            zdc_hir::url_is_safe(url),
            safe,
            "zdc_hir::url_is_safe disagrees with the table on {url:?}"
        );
        assert_eq!(
            safe_url_accepts(&mut context, url),
            safe,
            "runtime/dom.js's safeUrl disagrees with the table on {url:?}"
        );
    }
}

/// The allowlists themselves, not merely their consequences on a table.
///
/// A scheme added to one side and not the other is caught by the cases
/// above only if somebody also remembers to add a case. This is caught
/// either way, and it is derived from the one Rust set rather than from a
/// second copy of it written here.
#[test]
fn the_rust_and_javascript_allowlists_hold_the_same_schemes() {
    let mut context = context(false);
    let listed = context
        .eval(Source::from_bytes(
            b"URL_SCHEMES.slice().sort().join(',')".as_slice(),
        ))
        .expect("URL_SCHEMES is in scope")
        .to_string(&mut context)
        .expect("a string")
        .to_std_string_escaped();

    let mut expected = zdc_hir::URL_SCHEMES.to_vec();
    expected.sort_unstable();
    assert_eq!(
        listed,
        expected.join(","),
        "runtime/dom.js and zdc_hir have drifted apart on which schemes are allowed"
    );
}

/// `safeUrl` is reached by values that are not strings, because a binding
/// returns whatever the program's expression evaluated to. It must not
/// throw, and it must not turn `null` into the text `null` in an `href`.
#[test]
fn a_missing_url_becomes_the_empty_string_rather_than_a_thrown_error() {
    let mut context = context(false);
    for expression in ["null", "undefined"] {
        let script = format!("safeUrl({expression}) === '' ? 'yes' : 'no'");
        let verdict = context
            .eval(Source::from_bytes(script.as_bytes()))
            .unwrap_or_else(|e| panic!("safeUrl({expression}) threw: {e}"))
            .to_string(&mut context)
            .expect("a string")
            .to_std_string_escaped();
        assert_eq!(
            verdict, "yes",
            "safeUrl({expression}) is not the empty string"
        );
    }
}
