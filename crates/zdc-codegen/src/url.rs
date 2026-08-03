//! Which URL schemes a value may name — one set, for every value that
//! becomes a URL a browser acts on.
//!
//! There were two. The markdown renderer admitted `http`, `https`,
//! `mailto` and `ftp`; the element table and `runtime/dom.js` admitted
//! `http`, `https`, `mailto` and `tel`. Neither admitted a scripting
//! scheme, so the disagreement was not a hole — it is how one appears
//! later, when a scheme is added to whichever set the reader happened to
//! open. Two closed sets answering one question is one set too many.
//!
//! Stated as an **allowlist** rather than as a list of the schemes known
//! to be dangerous. `javascript:`, `data:` and `vbscript:` are the three
//! usually named, but which schemes a browser will execute or navigate to
//! is the browser's decision and the set grows; a denylist is out of date
//! the day it is written.
//!
//! §16.3.5's escaping argument does not cover any of this. That argument
//! is about *markup* parsing — it establishes that `&`, `<` and `>` cannot
//! close a tag or open one. An `href` is handed to the URL parser instead:
//! `setAttribute('href', v)` stores `v` verbatim and the browser runs
//! `javascript:alert(1)` on click, escaped or not, because there is
//! nothing in it for an HTML escaper to touch.
//!
//! `safeUrl` in `runtime/dom.js` is the runtime half of the same rule, for
//! the values that are not known until the program runs.
//! `crates/zdc-codegen/tests/url.rs` runs both halves against one table in
//! a real JavaScript engine, and compares the two lists directly, so a
//! scheme added to one side and not the other fails the build.

/// The schemes a URL-bearing attribute or a rendered link may name.
///
/// Everything else is refused. A URL with no scheme at all — `/work`,
/// `./a.png`, `#top` — is relative, which is the commonest URL in a
/// program, and is always allowed.
///
/// Four members, each with a reason to be here:
///
/// * `http` and `https` are the web.
/// * `mailto` hands the value to the reader's mail client. It opens no
///   document and runs no script, and a contact link is something every
///   page in the examples directory has.
/// * `tel` is the same argument for the dialer, and it is the spelling a
///   phone number takes in markup. It is also what `safeUrl` already
///   admits, so dropping it would blank links the runtime accepts today
///   while the compiler said nothing.
///
/// `ftp` was in the markdown half alone and is **not** here. It is not a
/// scripting scheme and was never a hole; it is simply dead. No shipping
/// browser has resolved an `ftp:` URL since 2021 — Chrome removed it in
/// 88 and Firefox in 90 — so admitting it emits a link no reader can
/// follow. It arrived from a CommonMark renderer's conventional list
/// rather than from a decision anyone made about this language, which is
/// exactly the kind of member a single reviewed set exists to keep out.
pub const URL_SCHEMES: &[&str] = &["http", "https", "mailto", "tel"];

/// The scheme `url` names, or `None` when it is relative.
///
/// Leading whitespace is stripped because a browser strips it before
/// parsing, so `\njavascript:alert(1)` is a `javascript:` URL. A colon
/// that appears after a `/`, `?` or `#` is inside a path or a query
/// rather than a scheme, so `/a:b` is relative and `notes/a:b.md` is a
/// file whose name contains a colon.
pub fn url_scheme(url: &str) -> Option<&str> {
    let trimmed = url.trim_start();
    let colon = trimmed.find(':')?;
    let scheme = &trimmed[..colon];
    if scheme.contains(['/', '?', '#']) {
        return None;
    }
    Some(scheme)
}

/// Whether a scheme is one of [`URL_SCHEMES`].
///
/// Compared ASCII-case-insensitively, because `JavaScript:` is the same
/// scheme as `javascript:` to the URL parser and therefore to the
/// attacker.
pub fn scheme_is_permitted(scheme: &str) -> bool {
    URL_SCHEMES
        .iter()
        .any(|permitted| scheme.eq_ignore_ascii_case(permitted))
}

/// Whether `url` may be emitted into an attribute the browser acts on.
pub fn url_is_safe(url: &str) -> bool {
    match url_scheme(url) {
        None => true,
        Some(scheme) => scheme_is_permitted(scheme),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_url_names_no_scheme() {
        assert_eq!(url_scheme(""), None);
        assert_eq!(url_scheme("/work"), None);
        assert_eq!(url_scheme("./a.png"), None);
        assert_eq!(url_scheme("#top"), None);
        // A colon inside a path or a query is not a scheme.
        assert_eq!(url_scheme("/a:b"), None);
        assert_eq!(url_scheme("notes/a:b.md"), None);
        assert_eq!(url_scheme("?q=a:b"), None);
        assert!(url_is_safe("/work"));
        assert!(url_is_safe("https://example.com/a:b"));
    }

    #[test]
    fn script_bearing_urls_are_refused() {
        assert!(!url_is_safe("javascript:alert(1)"));
        assert!(!url_is_safe("JavaScript:alert(1)"));
        assert!(!url_is_safe("  javascript:alert(1)"));
        assert!(!url_is_safe("data:text/html,<script>"));
        assert!(!url_is_safe("vbscript:msgbox(1)"));
    }

    /// The two entries the two sets used to disagree about, pinned in the
    /// direction the merged set settled them.
    #[test]
    fn the_disputed_schemes_are_settled_one_way_each() {
        assert!(url_is_safe("tel:+441234567890"));
        assert!(url_is_safe("mailto:a@example.com"));
        assert!(!url_is_safe("ftp://example.com/f"));
    }

    /// Membership is the list's, not a copy of it: every member is
    /// admitted, and a scheme that is not a member is not.
    #[test]
    fn every_listed_scheme_is_admitted_and_nothing_else_is() {
        for scheme in URL_SCHEMES {
            assert!(
                url_is_safe(&format!("{scheme}:rest")),
                "`{scheme}` is listed and must be admitted"
            );
            assert!(
                url_is_safe(&format!("{}:rest", scheme.to_uppercase())),
                "`{scheme}` must be admitted whatever its case"
            );
        }
        for scheme in ["file", "blob", "ws", "ftp", "unknown-scheme"] {
            assert_eq!(
                url_is_safe(&format!("{scheme}:rest")),
                URL_SCHEMES.contains(&scheme),
                "`{scheme}` must be admitted exactly when it is listed"
            );
        }
    }
}
