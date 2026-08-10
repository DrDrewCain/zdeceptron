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

/// The schemes a URL-bearing attribute or a rendered link may name.
///
/// Anything else — and, in particular, anything that executes — is
/// refused. A URL with no scheme at all is relative, which is the
/// commonest case in a program and is always allowed.
///
/// **There were two of these sets.** The markdown renderer admitted
/// `http`, `https`, `mailto` and `ftp`; the element table and
/// `runtime/dom.js` admitted `http`, `https`, `mailto` and `tel`. Neither
/// admitted a scripting scheme, so the disagreement was not a hole — it is
/// how one appears later, when a scheme is added to whichever set the
/// reader happened to open. Two closed sets answering one question is one
/// set too many, so this is the one, and it lives in `zdc-hir` because
/// `zdc-graph` and `zdc-codegen` both read it and neither depends on the
/// other.
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
/// browser has resolved an `ftp:` URL since 2021 — Chrome removed it in 88
/// and Firefox in 90 — so admitting it emits a link no reader can follow.
/// It arrived from a CommonMark renderer's conventional list rather than
/// from a decision anyone made about this language, which is exactly the
/// kind of member a single reviewed set exists to keep out.
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

/// Where a `request` declaration's destination sends the browser.
///
/// Two answers and no third, because the browser only has two: a path on
/// the page's own origin, which `connect-src 'self'` already permits, or
/// an origin written out in full, which the emitted policy has to name
/// before the browser will allow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// An absolute path on the page's own origin — `/data/quote.json`.
    SameOrigin,
    /// `scheme://host[:port]`, lowercased, with no path, query or
    /// fragment. Exactly what a `connect-src` source expression is.
    CrossOrigin(String),
}

/// Read a `request` destination, or say why it is not one.
///
/// **This is deliberately stricter than [`url_is_safe`], which governs a
/// different question.** An attribute's URL is dereferenced by the browser
/// however it likes and a relative one resolves against whatever document
/// it landed in; a request destination is a host the compiler writes into
/// a Content-Security-Policy, so it has to be decidable *here*, from the
/// literal alone, with no document in hand. Five refusals follow from
/// that, and each of them is a URL some browser would happily fetch:
///
/// * **A scheme that is not `http` or `https`.** The admitted set is
///   [`URL_SCHEMES`] filtered to the two that fetch — `mailto:` and `tel:`
///   hand the value to another program and fetch nothing, so a request to
///   one is a request that never happens. Everything outside the set is
///   refused for the reason [`url_is_safe`] refuses it, and a `data:` or
///   `javascript:` destination is refused twice over.
/// * **A protocol-relative `//host/path`.** It is cross-origin, and its
///   scheme is whatever the page happened to be served over. `url_scheme`
///   reads no scheme from it, so a rule written in terms of schemes alone
///   would file it under "relative" and let it through unnamed.
/// * **A path with no leading `/`.** `notes.json` resolves against the
///   *document*, and a routed program serves one program from many
///   documents, so the same declaration would fetch a different URL per
///   page. Same-origin is spelled with a leading slash and nothing else.
/// * **Userinfo.** `https://key@host/` puts a credential in a URL, which
///   is the thing this whole feature exists to keep out of one.
/// * **Whitespace or a control character anywhere in it.** A tab inside
///   `java\tscript:` is the classic way round a scheme check, and a
///   destination is a literal the author typed rather than a value that
///   arrived from somewhere, so there is never a reason for one.
///
/// A query or a fragment written into the destination is refused too: the
/// query is what `with` produces, and two ways to spell one thing is the
/// §4.1 violation this language is arranged against.
pub fn destination(url: &str) -> Result<Destination, &'static str> {
    if url.is_empty() {
        return Err("it is empty");
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("it holds a space or a control character");
    }
    if url.contains(['?', '#']) {
        return Err("a query or a fragment belongs in `with`, not in the destination");
    }
    if url.starts_with("//") {
        return Err("`//` leaves the scheme to whatever served the page; write it out");
    }
    let Some(scheme) = url_scheme(url) else {
        return if url.starts_with('/') {
            Ok(Destination::SameOrigin)
        } else {
            Err("a same-origin destination starts with `/`")
        };
    };
    if !FETCHING_SCHEMES
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
    {
        return Err("only `http:` and `https:` fetch");
    }
    let rest = &url[scheme.len()..];
    let Some(authority) = rest.strip_prefix("://") else {
        return Err("a scheme is followed by `://` and a host");
    };
    let authority = match authority.find('/') {
        Some(at) => &authority[..at],
        None => authority,
    };
    if authority.is_empty() {
        return Err("there is no host after the scheme");
    }
    if authority.contains('@') {
        return Err("a credential does not belong in a URL");
    }
    if !authority
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
    {
        return Err("the host holds a character a host cannot hold");
    }
    Ok(Destination::CrossOrigin(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase()
    )))
}

/// The members of [`URL_SCHEMES`] a browser issues an HTTP request for.
///
/// A **subset**, and `fetching_schemes_are_a_subset_of_the_permitted_ones`
/// is what keeps it one: `mailto` and `tel` hand the value to another
/// program and produce no request, so a destination naming one is a
/// request that never happens. A scheme added to [`URL_SCHEMES`] does not
/// arrive here on its own — that is the point. Somebody has to decide
/// whether it fetches, and the test fails if this set ever names a scheme
/// the language does not permit at all.
pub const FETCHING_SCHEMES: &[&str] = &["http", "https"];

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

    #[test]
    fn fetching_schemes_are_a_subset_of_the_permitted_ones() {
        for scheme in FETCHING_SCHEMES {
            assert!(
                URL_SCHEMES.contains(scheme),
                "`{scheme}` fetches but is not a scheme the language permits at all"
            );
        }
        // And it is a proper subset: the two that hand the value to
        // another program are the ones a destination may not name.
        for scheme in ["mailto", "tel"] {
            assert!(URL_SCHEMES.contains(&scheme));
            assert!(!FETCHING_SCHEMES.contains(&scheme));
        }
    }

    #[test]
    fn a_same_origin_destination_is_an_absolute_path() {
        assert_eq!(destination("/data/quote.json"), Ok(Destination::SameOrigin));
        assert_eq!(destination("/"), Ok(Destination::SameOrigin));
    }

    #[test]
    fn a_cross_origin_destination_yields_the_origin_alone() {
        for (written, origin) in [
            (
                "https://api.example.org/v1/search",
                "https://api.example.org",
            ),
            ("http://localhost:8000/quote.json", "http://localhost:8000"),
            // The origin is what a policy names, so it is normalised: a
            // host differing only in case is the same host, and a policy
            // that spelled it twice would allow one and not the other.
            ("HTTPS://API.Example.ORG/x", "https://api.example.org"),
            // No path at all is still an origin.
            ("https://api.example.org", "https://api.example.org"),
        ] {
            assert_eq!(
                destination(written),
                Ok(Destination::CrossOrigin(origin.to_string())),
                "`{written}`"
            );
        }
    }

    /// Every destination a browser would fetch and the compiler will not.
    ///
    /// Each line is a URL that works in a browser, which is what makes
    /// the list worth having: a refusal of something nothing could fetch
    /// would prove nothing.
    #[test]
    fn the_destinations_that_cannot_be_written_down_are_refused() {
        for url in [
            // Cross-origin with the scheme left to the page.
            "//api.example.org/v1",
            // Resolves against the document, and a routed program has many.
            "quote.json",
            "./quote.json",
            // A credential in a URL.
            "https://key@api.example.org/v1",
            // The query is what `with` writes.
            "/search?q=x",
            "https://api.example.org/v1#frag",
            // The scheme check, and the usual ways round one.
            "javascript:alert(1)",
            "data:text/plain,x",
            "ftp://example.com/f",
            "ws://example.com/s",
            "file:///etc/hosts",
            "mailto:a@example.com",
            "tel:+441234567890",
            " /quote.json",
            "/quo\tte.json",
            // A scheme with no authority after it.
            "https:/quote.json",
            "https://",
            "",
        ] {
            assert!(
                destination(url).is_err(),
                "`{url}` must not be a request destination"
            );
        }
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
