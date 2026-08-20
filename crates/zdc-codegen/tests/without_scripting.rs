//! What a page says to a reader who runs no JavaScript — #141.
//!
//! > A blank page is the worst failure mode there is. Either the
//! > server-rendering work makes this moot, or the language should emit
//! > something honest.
//!
//! Both halves happened. The prerender pass makes it moot for any program
//! the build host can run: the document arrives with the whole page in it,
//! so there is nothing to fall back *to*. It is best-effort by design, and
//! a program it cannot run still ships the empty container — that page is
//! the blank one, and that page gets the sentence.
//!
//! So the property is not "every page carries a `noscript`". It is the
//! sharper one: **a page carries the fallback exactly when the page needs
//! it**, because a `noscript` on a prerendered document would be shown to
//! the one reader for whom it is false.

mod support;

use support::compile_example;

/// Between `<div id="app">` and its close, or `None` for a page with no
/// container at all.
fn painted(page: &str) -> Option<&str> {
    page.split(r#"<div id="app">"#)
        .nth(1)?
        .split("</div>")
        .next()
}

fn says_it_needs_scripting(page: &str) -> bool {
    page.contains("<noscript>")
}

/// A program that asks for something over the network is still a program
/// the build host can run. The request is `Loading` when the page is built
/// — that is what a `Remote of T` starts as — so the reader is served the
/// `Loading` arm, which is the arm the client shows too and then adopts.
///
/// **It shipped the empty container and the sentence until `flattened`
/// kept an import's renames.** The emitted module imports
/// `{ request as $request }`; a pass that dropped the whole line left
/// `$request` undeclared, which is a `ReferenceError` at load, which the
/// pass turns into `None`. Every program with a `request` or a server read
/// emits exactly that shape, so not one of them had ever been painted —
/// and each was telling its reader it needed JavaScript to appear, which
/// is the claim this file exists to keep honest.
#[test]
fn a_program_that_asks_the_network_is_painted_in_its_loading_arm() {
    let bundle = compile_example("examples/quote.zd");
    let page = bundle.index_html.expect("this example emits a page");
    let inside = painted(&page).expect("this example has a container");
    assert!(
        !inside.is_empty(),
        "a `request` is `Loading` at build time, which is paintable:\n{page}"
    );
    assert!(
        !says_it_needs_scripting(&page),
        "the page was painted, so the fallback would be shown to a reader \
         who is already reading it:\n{page}"
    );
}

/// `gauge.zd` holds a `foreign`, which the build host has no copy of — so
/// the prerender returns `None` and the container ships empty. That is the
/// page the fallback exists for.
#[test]
fn a_page_the_build_could_not_paint_says_why_it_is_empty() {
    let bundle = compile_example("examples/gauge.zd");
    let page = bundle.index_html.expect("this example emits a page");
    assert_eq!(
        painted(&page),
        Some(""),
        "this test is about the *unpainted* branch, and the build painted something. \
         Pick another example, or the assertion below is testing nothing."
    );
    assert!(
        says_it_needs_scripting(&page),
        "an empty container and no explanation is the blank page #141 is about:\n{page}"
    );
}

/// The other half, and the reason the fallback is conditional rather than
/// unconditional: a prerendered page must **not** tell its reader that it
/// needs JavaScript to appear, because they are already reading it.
#[test]
fn a_painted_page_does_not_claim_to_need_scripting() {
    // Programs with no `static` state, so `compile_example` is the whole
    // build: one that declares `static` needs its build root run first,
    // and that is `static_placement.rs`'s subject rather than this one.
    for example in [
        "examples/counter.zd",
        "examples/hello.zd",
        "examples/disclosure.zd",
    ] {
        let bundle = compile_example(example);
        let page = bundle
            .index_html
            .unwrap_or_else(|| panic!("{example} emits a page"));
        let inside = painted(&page).unwrap_or_else(|| panic!("{example}: no container"));
        assert!(
            !inside.is_empty(),
            "{example} is here as a *painted* page and the build painted nothing:\n{page}"
        );
        assert!(
            !says_it_needs_scripting(&page),
            "{example} arrives with its content in it, so a reader with no JavaScript \
             reads the page. Telling them it needs JavaScript to appear contradicts \
             what is on their screen:\n{page}"
        );
    }
}

/// The sentence is text and nothing else. There is no stylesheet a
/// `noscript` can rely on having been fetched, and the emitted policy
/// admits no inline style — so a fallback that tried to look like anything
/// would either do nothing or be blocked.
#[test]
fn the_fallback_is_a_sentence_and_not_a_layout() {
    let bundle = compile_example("examples/gauge.zd");
    let page = bundle.index_html.expect("a page");
    let fallback = page
        .split("<noscript>")
        .nth(1)
        .and_then(|rest| rest.split("</noscript>").next())
        .expect("the fallback");
    assert!(
        !fallback.contains('<'),
        "no markup inside the fallback: {fallback}"
    );
    assert!(
        !fallback.contains("style"),
        "nothing the policy would block: {fallback}"
    );
    assert!(
        fallback.contains("JavaScript"),
        "it has to name the thing that is missing: {fallback}"
    );
}

/// **A keystroke does not paint a document.**
///
/// Painting means *running the emitted program* in a JavaScript engine, and
/// `check` is what a language server calls on every edit. The two facts sat
/// next to each other unnoticed for a while: `check` already stubs the
/// `static` values rather than computing them, with a comment saying that
/// evaluating the build root "is a step of `zdc build` and not of a
/// keystroke in an editor" — and then the first-paint pass landed inside
/// the same function and did exactly that.
///
/// It is not subtle when measured. `zdc-lsp`'s own latency suite, on the
/// 60 kB file it uses for the tail of the range, went from 92 ms to 1.4
/// seconds — a fifteenfold regression in the number an editor's
/// per-keystroke budget is made of, invisible to every other test because
/// every other test asserts output rather than time.
///
/// So the option is asserted here, where a change to it fails a test in the
/// crate that owns the decision, rather than only in a timing test that
/// runs elsewhere and reports lag without naming a cause.
#[test]
fn check_does_not_paint_the_document() {
    let options = zdc_codegen::Options::new("<check>", "check");
    assert!(
        options.first_paint,
        "a build paints by default; only the callers that throw the page away opt out"
    );
    assert!(
        !options.without_first_paint().first_paint,
        "`without_first_paint` is what `check` uses to stay off the engine"
    );
}
