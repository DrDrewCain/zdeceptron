//! A page a content site would serve, evaluated in the embedded engine.
//!
//! `examples/page.zd` is the acceptance test for the widened element
//! vocabulary. Asserting about the emitted source would only restate the
//! shape table; what has to be true is that the *document* comes out right,
//! so this mounts the module against the DOM shim and reads the tree back —
//! which is what a browser would have shown.
//!
//! Three claims, each of which was unreachable before:
//!
//!   1. The document has landmarks, an outline, real anchors, a real image
//!      and real list semantics.
//!   2. The outline is `h1` then `h2`, and nothing in the program says so:
//!      the level is the nesting depth, so it cannot be written wrongly.
//!   3. A link built from data by `each` is still a real `href`.

mod support;

use support::{check_refusals, compile_example, context, refusals, run};

/// Mount the page, then open the disclosure, returning both frames.
const DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
const $frames = [serialize($host)];
walk($host).find((n) => n.tagName === 'button').fire('click');
$frames.push(serialize($host));
"#;

fn frames() -> Vec<String> {
    let module = compile_example("examples/page.zd").client_js;
    let mut context = context(false);
    run(
        &mut context,
        &module,
        &format!("{DRIVER}\n$frames.join('\\u0001')"),
    )
    .split('\u{1}')
    .map(str::to_string)
    .collect()
}

#[test]
fn the_page_renders_the_document_a_content_site_needs() {
    let frames = frames();
    let page = &frames[0];

    // Landmarks. None of these five tags was producible.
    for landmark in ["<main>", "<nav>", "<article>", "<section>", "<footer>"] {
        assert!(page.contains(landmark), "no {landmark} in:\n{page}");
    }

    // Prose.
    assert!(page.contains(
        "<p>Short pieces about compilers and the interfaces people build on top of them.</p>"
    ));
    assert!(page.contains("<em>Everything here is one file.</em>"));
    assert!(page.contains("<code>zdc</code>"));
    assert!(page.contains(r#"<time datetime="2026-08-03">3 August 2026</time>"#));

    // A list with list semantics, not a stack of divs.
    assert!(page.contains("<ul><li>"), "no list in:\n{page}");

    // A hyperlink — the thing a portfolio is made of, and the element
    // §14G.2's own milestone-7 example writes without one existing.
    assert!(
        page.contains(r#"<a href="https://example.com/feed.xml">"#),
        "no external link in:\n{page}"
    );

    // An image, with the text alternative the compiler requires.
    assert!(
        page.contains(r#"alt="A desk with a terminal open on a compiler error""#),
        "no described image in:\n{page}"
    );
    assert!(page.contains("<figure>") && page.contains("<figcaption>"));
}

/// The level is the nesting depth. Nothing in `page.zd` names a level, so
/// an outline that starts at `h2` or skips to `h4` is not expressible.
#[test]
fn the_outline_starts_at_one_and_does_not_skip() {
    let page = frames().remove(0);

    assert!(page.contains("<h1>Field notes</h1>"), "{page}");
    assert!(page.contains("<h2>Recent</h2>"), "{page}");
    assert!(page.contains("<h2>Colophon</h2>"), "{page}");

    let levels: Vec<u32> = page
        .match_indices("<h")
        .filter_map(|(at, _)| page[at + 2..at + 3].parse().ok())
        .collect();
    assert_eq!(levels, [1, 2, 2], "the outline of the page:\n{page}");
    assert!(!page.contains("<h3>"), "nothing is three deep:\n{page}");
}

/// A row's `href` comes from its item, through the same `bindAttr` every
/// other attribute uses — and through `safeUrl`, so a URL that arrived from
/// data cannot be `javascript:`.
#[test]
fn a_link_built_by_each_is_a_real_anchor() {
    let page = frames().remove(0);
    assert!(
        page.contains(r#"<a href="/notes/signals">"#),
        "the first note's link:\n{page}"
    );
    assert!(
        page.contains("<span>Signals without a dependency array</span>"),
        "the first note's title:\n{page}"
    );
    assert!(
        page.contains(r#"<a href="/notes/placement">"#),
        "the second note's link:\n{page}"
    );
}

/// The page is not a static document: the disclosure is an ordinary
/// `if` over client state, and the description list it reveals is markup
/// the vocabulary could not previously produce either.
#[test]
fn the_page_still_reacts() {
    let frames = frames();
    assert!(!frames[0].contains("<dl>"), "closed:\n{}", frames[0]);
    assert!(
        frames[1].contains("<dl><dt>Compiler</dt><dd>Rust, no runtime dependencies</dd>"),
        "opened:\n{}",
        frames[1]
    );
}

/// The two structural rules the vocabulary carries, checked from the
/// element that breaks them rather than from the one that keeps them.
#[test]
fn an_orphaned_list_item_is_refused() {
    let messages = refusals("view\n    Column\n        Item \"one\"\n");
    assert!(
        messages
            .iter()
            .any(|m| m.contains("`Item`") && m.contains("`List`")),
        "{messages:?}"
    );

    let messages = refusals("view\n    List\n        Paragraph \"not an item\"\n");
    assert!(
        messages
            .iter()
            .any(|m| m.contains("`Paragraph` is not one")),
        "{messages:?}"
    );
}

/// The attribute set is closed. It was open, and every element was
/// therefore a place to write `onclick`.
#[test]
fn an_attribute_the_element_does_not_have_is_refused() {
    for source in [
        "view\n    Column onclick is \"alert(1)\"\n        Text \"x\"\n",
        "view\n    Paragraph \"x\", style is \"color:red\"\n",
        "view\n    Image source is \"/a.png\", alt is \"a\", srcdoc is \"<script>\"\n",
    ] {
        let messages = refusals(source);
        assert!(
            messages.iter().any(|m| m.contains("The set is closed")),
            "{source} was not refused: {messages:?}"
        );
    }
}

/// A URL written in the source is checked when it is written.
#[test]
fn a_link_that_would_run_script_is_refused() {
    let messages = refusals("view\n    Link \"javascript:alert(1)\"\n        Text \"go\"\n");
    assert!(
        messages
            .iter()
            .any(|m| m.contains("javascript:alert(1)") && m.contains("script execution")),
        "{messages:?}"
    );
}

/// A `secret` may not reach an attribute — and it does not, but not for
/// the reason it eventually should.
///
/// §14G.1.3(c) declares a **closed** sink list: client state, the view,
/// build artefacts, outbound HTTP response bodies, platform logs, and
/// live-sync streams. Attributes are new sinks that list does not name in
/// its own terms. An attribute is arguably "the view", but two of these
/// three cases are not:
///
///   * `Column id is key` puts a secret in the DOM where nothing renders
///     it. Reading "the view" as "what appears on screen" misses it.
///   * `Image source is key` and `Link key` make the **browser send a
///     request** carrying the secret in a URL. That is exfiltration, not
///     rendering, and the list has no sink for a browser-initiated
///     outbound request — sink 4 is a *response* body.
///
/// The sink list was not extended, because there is nothing to extend it
/// in: `zdc-graph` does not exist and codegen refuses any `secret` signal
/// outright rather than emit an unenforced guarantee. So all three are
/// compile errors today for a reason that has nothing to do with where the
/// value was going. That is the guarantee this test pins, stated honestly:
/// when the information-flow pass lands, these three must still be errors,
/// and the third of them needs a sink the specification has not declared.
#[test]
fn a_secret_reaching_an_attribute_is_a_compile_error() {
    for source in [
        // The attribute of an ordinary element.
        "secret state key is client Text starting \"sk\"\nview\n    Column id is key\n        Text \
         \"x\"\n",
        // A URL the browser will fetch.
        "secret state key is client Text starting \"sk\"\nview\n    Image source is key, alt is \
         \"a\"\n",
        // A URL the browser will fetch when clicked.
        "secret state key is client Text starting \"sk\"\nview\n    Link key\n        Text \"go\"\n",
    ] {
        let messages = refusals(source);
        assert!(
            messages.iter().any(|m| m.contains("is `secret`")),
            "a secret reached an attribute in:\n{source}\n{messages:?}"
        );
    }
}

/// An image with no text alternative is the commonest accessibility
/// failure there is, and a default would have silently produced one.
#[test]
fn an_image_must_describe_itself() {
    let messages = check_refusals("view\n    Image source is \"/a.png\"\n");
    assert!(
        messages.iter().any(|m| m.contains("`alt is …`")),
        "{messages:?}"
    );
}
