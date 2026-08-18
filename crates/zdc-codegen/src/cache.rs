//! What a host may cache, for how long, and which files earn it — #137.
//!
//! Without content hashing a deployment has two options and both are bad:
//! serve every file with a short lifetime, so every visitor re-fetches the
//! whole bundle, or serve it with a long one and hand a visitor a page
//! assembled from two different builds. This module is the third option,
//! and it is only available for a file whose *name* changes when its bytes
//! do.
//!
//! # The rule: a file is hashed when this compiler writes every reference
//! to it
//!
//! An emitted bundle names its own files in exactly two ways, and the
//! difference between them decides everything here.
//!
//! * **An href this compiler prints.** `<link rel="stylesheet" href=…>` in
//!   the document. The compiler writes the file *and* the only string that
//!   points at it, so it can rename both together and nothing else can
//!   disagree.
//! * **An ES import specifier.** `boot.js` imports `./client.js`,
//!   `client.js` imports `./runtime/dom.js`, and `runtime/dom.js` imports
//!   `./signal.js`. The first two are strings this compiler prints. **The
//!   third is not.** It is line 12 of `zdc-runtime/runtime/dom.js`, a
//!   hand-written module this crate ships byte for byte and never parses.
//!
//! So the answer for the module graph is one answer for the whole graph:
//! **it is not hashed, entry included.** Renaming a runtime module means
//! rewriting a specifier inside a hand-written file, which is a bundler
//! pass this compiler does not have and should not grow for a cache
//! header — and shipping those modules unmodified is what lets
//! `runtime/*.test.js` test the bytes that actually deploy.
//!
//! Hashing `client.js` alone was considered and rejected on its own terms
//! rather than for the cost. It would put `immutable` on the one file in
//! the graph that changes on *every* build, and leave it off the fourteen
//! that change only when the compiler is upgraded. The caching win lives in
//! the stable leaves, and the leaves are exactly the part that cannot be
//! renamed. The shape that would work is a hashed *directory* — sibling
//! imports are relative, so the whole tree can move together and every
//! specifier inside it stays true — and it belongs with whatever first has
//! to parse these modules for another reason (#135's minifier is the
//! natural place), not here.
//!
//! # The asset directory: stylesheets only
//!
//! A `.css` under `assets/` is reached by a `<link>` this compiler prints,
//! so it is hashed. Everything else there is not, and the reason is a line
//! in a checked-in example:
//!
//! ```text
//! Image source is "/assets/desk.png", alt is "…"
//! ```
//!
//! That URL is the *program's* text. So is `url(./Inter.woff2)` inside an
//! author's stylesheet. Renaming `desk.png` breaks both, and it breaks them
//! as a 404 for an image, which is the quietest failure in this crate —
//! nothing refuses, nothing warns, the page just has a gap. A compiler may
//! only rename a file it can prove it named.
//!
//! One reference an author *can* write to a stylesheet is `@import`, and
//! [`crate::assets::discover`] finds those and leaves their targets under
//! their own names for the same reason.
//!
//! The generated stylesheet is hashed under the same rule and with one
//! fewer thing to check: it is not in the asset directory, nothing copies
//! it, and the `<link>` in the document is the only string that has ever
//! named it.
//!
//! # Why the generated rules say only half of what is true
//!
//! [`headers`] emits a rule for the immutable files and no catch-all for
//! the rest, even though "everything else must revalidate" is equally true.
//! `_headers` is read by more than one host and they do not document the
//! same precedence for two rules matching one path: a `/*` rule beside an
//! exact one can end up as a single `Cache-Control: public, max-age=0,
//! must-revalidate, public, max-age=31536000, immutable`, where the first
//! `max-age` wins and the long-lived intent is silently inverted. Every
//! host that reads this file already defaults an un-ruled path to
//! revalidate-on-request, so the catch-all would buy nothing and could cost
//! the rule beside it.
//!
//! Where the compiler writes the serving code itself — Deno's entry reads
//! `public/` in JavaScript this crate's sibling generates — both halves can
//! be stated, because there is no second implementation's precedence to
//! guess at.
//!
//! # The old file left behind
//!
//! `zdc build` does not empty its output directory, so a second build
//! leaves the previous build's `styles.<hash>.css` beside the new one.
//! That is not a leak to tidy up later, it is the property that makes a
//! rolling deploy safe: a visitor holding the previous document asks for
//! the previous stylesheet, and a host that still has it answers correctly
//! instead of 404ing halfway through a release. A directory that is
//! emptied first has to be replaced atomically or it has a window in which
//! neither build is complete.

/// What a hashed file may be served with: one year, and the browser is
/// entitled not to ask again.
///
/// `immutable` is the load-bearing half. `max-age` alone still permits a
/// revalidation on reload, which is exactly when a visitor is most likely
/// to be waiting; `immutable` tells the browser that a conditional request
/// for this URL can never be answered with anything but 304, because the
/// URL is a function of the bytes.
///
/// A year rather than forever because 31536000 is the largest value the
/// HTTP specification's own guidance suggests, and a larger one is capped
/// by implementations anyway.
pub const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// What everything else may be served with: cached, and checked every
/// time.
///
/// `max-age=0, must-revalidate` is a conditional request per load, which
/// costs a round trip and returns 304 with no body. That is the correct
/// price for a file whose URL does not change when its content does — the
/// document above all, since a stale `index.html` points at the previous
/// build's stylesheet and the page renders unstyled with nothing in the
/// console to say why.
pub const REVALIDATE: &str = "public, max-age=0, must-revalidate";

/// The `_headers` file for a bundle whose hashed files are `immutable`, or
/// `None` when there are none to talk about.
///
/// `_headers` rather than a bespoke format because it is the one that is
/// already read where these bundles land: Cloudflare Workers serves the
/// browser half through its static-assets binding, which reads a `_headers`
/// beside it, and Cloudflare Pages and Netlify read the same file with the
/// same syntax. It is written for those; a host that reads neither ignores
/// a file it does not know, which is the failure mode a generated config
/// should have.
///
/// `None` rather than an empty file: a configuration that grants nothing is
/// a file a reader has to open to discover it says nothing.
pub fn headers(immutable: &[String]) -> Option<String> {
    if immutable.is_empty() {
        return None;
    }
    let mut out = String::from(
        "# zdc · generated, do not edit.\n\
         #\n\
         # Every path below carries a content hash in its name, so its URL\n\
         # changes whenever its bytes do and an old URL is never reused. That\n\
         # is what makes a year of `immutable` safe rather than a way to serve\n\
         # a visitor last week's stylesheet.\n\
         #\n\
         # Nothing else is listed. A path with no rule here keeps the host's\n\
         # default, which is to revalidate on every request — the right answer\n\
         # for `index.html`, for the runtime modules, and for any asset a\n\
         # program names itself.\n",
    );
    // Sorted, so two builds of the same program produce the same file. The
    // caller assembles this list from two sources — the emitter and the
    // asset walk — and neither ordering is one a reader should have to
    // know about.
    let mut paths: Vec<&String> = immutable.iter().collect();
    paths.sort();
    paths.dedup();
    for path in paths {
        out.push_str(&format!("\n/{path}\n  Cache-Control: {IMMUTABLE}\n"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_with_nothing_hashed_gets_no_file() {
        assert_eq!(headers(&[]), None);
    }

    #[test]
    fn every_hashed_path_gets_the_long_lived_rule_and_nothing_else_does() {
        let text = headers(&[
            "styles.0123456789abcdef.css".to_string(),
            "assets/site.fedcba9876543210.css".to_string(),
        ])
        .expect("two hashed files, so a file");
        assert!(text.contains("\n/styles.0123456789abcdef.css\n  Cache-Control: public, max-age=31536000, immutable\n"));
        assert!(text.contains("\n/assets/site.fedcba9876543210.css\n  Cache-Control: public, max-age=31536000, immutable\n"));
        assert!(
            !text.contains("/*"),
            "no catch-all: two rules matching one path is a merge whose result \
             is the host's business, not ours\n{text}"
        );
        assert_eq!(
            text.matches("Cache-Control").count(),
            2,
            "one rule per hashed file\n{text}"
        );
    }

    /// The list arrives from two places, so the file cannot depend on which
    /// one spoke first.
    #[test]
    fn the_rules_are_sorted_so_two_builds_agree() {
        let one = headers(&["b.1.css".to_string(), "a.2.css".to_string()]);
        let other = headers(&["a.2.css".to_string(), "b.1.css".to_string()]);
        assert_eq!(one, other);
        let text = one.expect("a file");
        assert!(text.find("/a.2.css") < text.find("/b.1.css"), "{text}");
    }
}
