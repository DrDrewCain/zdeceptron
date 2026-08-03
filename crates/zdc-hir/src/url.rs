//! URL positions: which arguments become a request, and which URLs run.
//!
//! Two rules live here because three passes need the same answer and a
//! second copy of either would be a soundness hole the moment it drifted.
//!
//! * **Which named arguments the browser dereferences.** §14G.1.3(c)'s
//!   sink 7 — the outbound request — is defined by this list. `zdc-graph`
//!   reads it to decide where a secret may not go; `zdc-codegen` reads it
//!   to decide where a value must be filtered.
//! * **Which URLs are fetched and which are executed.** §16.3.5's escaping
//!   argument is about *markup* parsing: it establishes that `&`, `<` and
//!   `>` cannot close a tag or open one. That argument says nothing about
//!   an attribute the browser hands to the URL parser rather than to the
//!   HTML parser. `setAttribute('href', v)` stores `v` verbatim, and
//!   `javascript:alert(1)` in an `href` executes on click — escaped or
//!   not, because there is nothing in it to escape. Escaping for HTML text
//!   is not escaping for a URL, and conflating the two is the classic
//!   cross-site-scripting mistake.
//!
//! The scheme rule is stated as an **allowlist**, not as a list of the
//! schemes known to be dangerous. `javascript:`, `data:` and `vbscript:`
//! are the three usually named, but the set of schemes a browser will
//! execute or navigate to is decided by the browser and grows; a denylist
//! is out of date the day it is written.
//!
//! `runtime/dom.js`'s `safeUrl` is the runtime half of the same rule, for
//! the values that are not literals. `crates/zdc-codegen/tests/url.rs`
//! runs the two against the same table in a real JavaScript engine, so a
//! change to one that is not made to the other fails the build.

/// Every named argument whose value the browser resolves and fetches,
/// navigates to, or executes.
///
/// Closed and sorted. Enforcement ranges over the **name**, on every
/// element, rather than over the elements that were meant to have one:
/// `zdc-codegen`'s `named_argument` passes an unrecognised name through as
/// the attribute of that name, so `Text src is apiKey` reaches the DOM
/// even though `Text` has no `src` in its signature. A rule keyed on the
/// element would let that through; this one does not.
///
/// `source` is the ZDeceptron spelling of `src` and `src` is the spelling
/// that reaches the DOM. Both are here, because a program may write
/// either.
///
/// `style` is here for CSS `url()`, which is a request the browser issues
/// from a value that never looks like a URL to the reader. It is a sink
/// rather than a filtered attribute: `zdc-codegen` refuses a `style`
/// argument outright, because filtering CSS is not filtering a URL either.
pub const URL_ATTRIBUTES: &[&str] = &[
    "action",
    "background",
    "cite",
    "codebase",
    "data",
    "formaction",
    "href",
    "icon",
    "longdesc",
    "manifest",
    "ping",
    "poster",
    "profile",
    "source",
    "src",
    "srcset",
    "style",
    "usemap",
];

/// Whether a named argument's value becomes a URL the browser dereferences.
pub fn is_url_attribute(name: &str) -> bool {
    URL_ATTRIBUTES.contains(&name)
}

/// The schemes a URL-bearing attribute may name.
///
/// Anything else — and, in particular, anything that executes — is
/// refused. A URL with no scheme at all is relative, which is the
/// commonest case in a program and is always allowed.
pub const URL_SCHEMES: &[&str] = &["http", "https", "mailto", "tel"];

/// The scheme of `url`, or `None` if it is relative.
///
/// Mirrors `safeUrl` in `runtime/dom.js` exactly: leading whitespace is
/// stripped, because a browser strips it before parsing and
/// `\njavascript:alert(1)` is therefore a `javascript:` URL; and a colon
/// that appears after a `/`, `?` or `#` is inside a path or a query rather
/// than a scheme, so `/a:b` is relative and not a `/a` scheme.
pub fn url_scheme(url: &str) -> Option<&str> {
    let trimmed = url.trim_start();
    let colon = trimmed.find(':')?;
    let scheme = &trimmed[..colon];
    if scheme.contains(['/', '?', '#']) {
        return None;
    }
    Some(scheme)
}

/// Whether this URL may be emitted into an attribute the browser
/// dereferences.
pub fn url_is_safe(url: &str) -> bool {
    match url_scheme(url) {
        None => true,
        Some(scheme) => URL_SCHEMES
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed)),
    }
}

/// Whether a named argument would install an event handler as an
/// attribute.
///
/// `on click` is the language's event syntax and it is a node, not an
/// argument. Anything spelled `on…` in argument position would reach the
/// DOM as an inline handler, which is a script the program did not write
/// in a position the compiler cannot see into.
pub fn is_event_attribute(name: &str) -> bool {
    name.len() > 2 && name.as_bytes()[..2].eq_ignore_ascii_case(b"on")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attribute_list_is_sorted_and_has_no_duplicates() {
        let mut sorted = URL_ATTRIBUTES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, URL_ATTRIBUTES.to_vec());
    }

    #[test]
    fn a_relative_url_has_no_scheme() {
        assert_eq!(url_scheme("/notes/signals"), None);
        assert_eq!(url_scheme("notes.html"), None);
        // A colon inside a path is not a scheme.
        assert_eq!(url_scheme("/a:b"), None);
        assert_eq!(url_scheme("?q=a:b"), None);
        assert!(url_is_safe("/notes/signals"));
    }

    #[test]
    fn the_three_executing_schemes_are_refused() {
        for url in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "  \n javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
        ] {
            assert!(!url_is_safe(url), "`{url}` must not be emissible");
        }
    }

    #[test]
    fn the_schemes_a_page_actually_uses_are_allowed() {
        for url in [
            "https://example.com/feed.xml",
            "HTTPS://example.com",
            "http://example.com",
            "mailto:someone@example.com",
            "tel:+441234567890",
        ] {
            assert!(url_is_safe(url), "`{url}` must be emissible");
        }
    }

    #[test]
    fn both_spellings_of_the_image_source_are_sinks() {
        assert!(is_url_attribute("source"));
        assert!(is_url_attribute("src"));
        assert!(is_url_attribute("srcset"));
        assert!(!is_url_attribute("alt"));
        assert!(!is_url_attribute("id"));
        assert!(!is_url_attribute("class"));
    }

    #[test]
    fn an_inline_handler_is_recognised_whatever_its_case() {
        assert!(is_event_attribute("onclick"));
        assert!(is_event_attribute("onError"));
        assert!(is_event_attribute("ONLOAD"));
        // `on` alone is not an attribute, and neither is a word that
        // merely starts with those letters and is one of ours.
        assert!(!is_event_attribute("on"));
        assert!(!is_event_attribute("o"));
    }
}
