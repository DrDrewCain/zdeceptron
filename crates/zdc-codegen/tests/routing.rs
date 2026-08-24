//! Per-page emission: one document per URL, and one page's code out of
//! another page's bundle.
//!
//! §14A's whole argument is bundle size. A site that ships every page's
//! code to every visitor has forfeited it, so "the bundles differ" is not
//! a nicety here — it is the claim under test.

mod support;

use support::{context, run};

use zdc_codegen::{compile_site, Options, PageBundle, SiteBundle};

/// The example, compiled through the module loader — `site.zd` imports
/// `content.zd`, and a file with a `use` line is not a whole program on
/// its own (§14D.2).
fn site_example() -> SiteBundle {
    build(&support::repository_path("examples/site.zd"))
}

fn build(path: &std::path::Path) -> SiteBundle {
    let linked = zdc_resolve::load(path)
        .unwrap_or_else(|failure| panic!("load: {}", failure.errors[0].message));
    let hir = zdc_resolve::Resolver::linked(&linked)
        .resolve()
        .unwrap_or_else(|errors| panic!("resolve: {}", errors[0].message));
    // The same pipeline `zdc build` runs, in §17.1.2's order: the split
    // first, then the checker and the flow pass against it.
    let split = zdc_graph::split(&hir);
    let verdict = zdc_graph::ifc(&hir, &split);
    let types = zdc_types::check(&hir, &split)
        .unwrap_or_else(|errors| panic!("check: {}", errors[0].message));
    let options = Options::new(path.display().to_string(), "site");
    let cleared = verdict
        .clearance()
        .unwrap_or_else(|| panic!("flow: {}", verdict.diagnostics[0].message));
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &types,
        cleared,
    };
    // §17.4.8's build root runs first, exactly as `zdc build` runs it: a
    // routed program enumerates its URLs from `static` state, so a test
    // that skipped this would be emitting against values nothing computed.
    let evaluated = match zdc_codegen::build_module(&inputs, &options)
        .unwrap_or_else(|errors| panic!("build root: {}", errors[0].message))
    {
        Some(module) => {
            let directory = path.parent().unwrap_or(std::path::Path::new("."));
            zdc_codegen::evaluate(&module, directory)
                .unwrap_or_else(|error| panic!("build root: {}", error.report()))
        }
        None => zdc_codegen::Evaluated::default(),
    };
    let options = options.with_statics(evaluated.values);
    compile_site(&inputs, &options).unwrap_or_else(|errors| {
        panic!(
            "emit:\n{}",
            errors
                .iter()
                .map(|e| format!("  {}", e.message))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

/// A document's module *and* the chunk it reads from, in load order.
///
/// A page used to be the whole of what a browser ran. It is not any
/// more: the definitions more than one route reaches are emitted once
/// into `pages/program.<hash>.js` and imported, so running a page's
/// module alone calls names nothing declared. The browser is handed both
/// and resolves the import; these tests have no module loader, so they
/// are handed both concatenated, which is what `prerender` does for the
/// same reason.
fn program(site: &SiteBundle, url: &str) -> String {
    match &site.program_chunk {
        Some(chunk) => format!("{}\n{}", chunk.client_js, page(site, url).client_js),
        None => page(site, url).client_js.clone(),
    }
}

fn page<'a>(site: &'a SiteBundle, url: &str) -> &'a PageBundle {
    site.pages
        .iter()
        .find(|page| page.url == url)
        .unwrap_or_else(|| {
            panic!(
                "no page at {url}; the site has {:?}",
                site.pages.iter().map(|p| &p.url).collect::<Vec<_>>()
            )
        })
}

#[test]
fn the_example_emits_one_document_per_url_plus_the_not_found_page() {
    let site = site_example();
    let urls: Vec<&str> = site.pages.iter().map(|page| page.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "/",
            "/writing",
            "/writing/routing",
            "/writing/folding",
            "/404"
        ],
        "the parameterised route contributes one document per enumerated value"
    );
}

/// **The acceptance criterion.** Per-route output actually differs, and
/// one page's code is not in another's.
#[test]
fn one_pages_code_is_not_in_another_pages_bundle() {
    let site = site_example();
    let home = &page(&site, "/").client_js;
    let index = &page(&site, "/writing").client_js;
    let routing = &page(&site, "/writing/routing").client_js;
    let folding = &page(&site, "/writing/folding").client_js;

    assert_ne!(home, index);
    assert_ne!(index, routing);
    assert_ne!(routing, folding);

    // The home page renders no post, so nothing that reads one is in it.
    assert!(
        !home.contains("titleOf"),
        "the home page must not carry the post helpers:\n{home}"
    );
    assert!(
        !home.contains("eachInto"),
        "the home page must not carry the index's list machinery:\n{home}"
    );
    assert!(
        !home.contains("A route is a choice"),
        "the home page must not carry a post's text:\n{home}"
    );

    // A helper only the home page reaches is in the home page and
    // nowhere else. This is §16.3.1's dead-code claim, discharged by the
    // closure walk rather than asserted.
    assert!(home.contains("tagline"), "{home}");
    for (url, emitted) in [
        ("/writing", index),
        ("/writing/routing", routing),
        ("/writing/folding", folding),
    ] {
        assert!(
            !emitted.contains("tagline"),
            "{url} carries a helper only the home page uses:\n{emitted}"
        );
    }

    // One post's document is bound to that post: the parameter is a
    // constant, so the lookup is a call with the value already in it.
    assert!(routing.contains("titleOf('routing')"), "{routing}");
    assert!(!routing.contains("titleOf('folding')"), "{routing}");
    assert!(folding.contains("titleOf('folding')"), "{folding}");
    assert!(!folding.contains("titleOf('routing')"), "{folding}");
}

/// What per-page splitting does **not** do, stated so nobody reads more
/// into it than is there.
///
/// A `function` is colorless and is emitted wherever it is reachable
/// from, so a helper two pages call is in both bundles whole — including
/// the branch the other page takes. Splitting is over the *view*, which
/// is where the route decides what runs; specialising a function body per
/// call site would be inlining, and this compiler does not inline.
#[test]
fn a_shared_helper_is_shared_whole_rather_than_specialised() {
    let site = site_example();
    // **Where it lives moved; what it is did not.** The helper is
    // reached by both entry documents, so it is emitted once into the
    // chunk rather than specialised into each — and it is still emitted
    // *whole*, with the branch this page does not take. That is the
    // claim: a shared definition is not narrowed to its caller.
    let shared = program(&site, "/writing/routing");
    assert!(
        shared.contains("An immutable signal has one value"),
        "the shared helper's other branch is part of the helper:\n{shared}"
    );
    assert!(
        !page(&site, "/")
            .client_js
            .contains("An immutable signal has one value"),
        "and a page that never calls it does not carry it in its own module"
    );
    // What the chunk costs, said out loud rather than left implied: the
    // home page does not *carry* the helper and does still *fetch* it,
    // because the chunk is one file. §16.3.1's claim is about what a
    // document is emitted with, and that is what is asserted above.
    if let Some(chunk) = &site.program_chunk {
        assert!(
            chunk
                .client_js
                .contains("An immutable signal has one value"),
            "the definition two pages reach is in the file they share"
        );
    }
}

/// §14G.2 revision 1's second consequence: the parameter is a constant
/// for each emitted document, so a lookup over it constant-folds.
/// `/writing/routing` inlines one post rather than the whole list.
#[test]
fn a_route_parameter_is_a_literal_in_the_document_it_belongs_to() {
    let site = site_example();
    let routing = &page(&site, "/writing/routing").client_js;
    assert!(
        routing.contains("titleOf('routing')"),
        "`slug` must be folded to its value:\n{routing}"
    );
    assert!(
        !routing.contains("slugs"),
        "a document that does not need the enumeration must not carry it:\n{routing}"
    );
}

/// The address signal is folded away entirely: no cell, no setter, no
/// `whenInto` dispatching on it at runtime.
#[test]
fn the_address_signal_costs_nothing_at_runtime() {
    let site = site_example();
    // `site.zd` declares four routes and a not-found page. A site that
    // emitted no document would satisfy the loop below over nothing.
    assert!(
        site.pages.len() >= 4,
        "the routed example emitted only {} documents",
        site.pages.len()
    );
    for page in &site.pages {
        assert!(
            !page.client_js.contains("whenInto"),
            "{}'s dispatch must be resolved at build time:\n{}",
            page.url,
            page.client_js
        );
        assert!(
            !page.client_js.contains("signal("),
            "{} allocates a cell for a value the document already knows:\n{}",
            page.url,
            page.client_js
        );
    }
}

/// `Link` renders a real anchor with a real `href`, which is the whole
/// argument of §14G.2 revision 1: every navigation is a document
/// navigation, so every navigation is crawlable.
#[test]
fn a_link_to_a_constant_route_is_baked_into_the_markup() {
    let site = site_example();
    let home = &page(&site, "/").client_js;
    // falsifiable: the emitter writes an attribute either into a
    // template literal, where the quote is escaped, or into a plain
    // string, where it is not — and which one it picks depends on how the
    // surrounding markup was built. Both arms name the same anchor with
    // the same URL, and a document that rendered the link as an effect
    // instead of as markup contains neither.
    assert!(
        home.contains("<a href=\\'/\\'") || home.contains("href=\"/\""),
        "the nav's links must be markup rather than effects:\n{home}"
    );
    assert!(home.contains("href=\"/writing\""), "{home}");
    assert!(
        !home.contains("setAttribute"),
        "a constant href costs nothing at runtime:\n{home}"
    );
}

/// A link whose parameter comes from a row's binder cannot be constant,
/// so it is a binding — one per row, reading that row's value.
#[test]
fn a_link_inside_a_list_binds_its_href() {
    let site = site_example();
    let index = &page(&site, "/writing").client_js;
    assert!(
        index.contains("bindAttr") && index.contains("'href'"),
        "a row's link must bind its href:\n{index}"
    );
    assert!(
        index.contains("'/writing' + '/' + String("),
        "the href must be rendered from the route rather than written by hand:\n{index}"
    );
}

/// The not-found document is the `None` arm of `when page` — the arm
/// exhaustiveness already forced the program to write. Routing adds no
/// construct for it.
#[test]
fn the_not_found_document_is_the_none_arm() {
    let site = site_example();
    let missing = page(&site, "/404");
    assert!(
        missing
            .client_js
            .contains("That URL is not part of this site"),
        "{}",
        missing.client_js
    );
    assert!(
        !missing.client_js.contains("A site in one file"),
        "the not-found page must not carry the home page:\n{}",
        missing.client_js
    );
}

/// The manifest is a build artefact derived from the `route`
/// declaration, never a file anyone writes (invariant 5).
#[test]
fn the_manifest_maps_every_url_to_its_module() {
    let site = site_example();
    let json = &site.routes_json;
    for (url, module) in [
        ("/", "/pages/index.js"),
        ("/writing", "/pages/writing.js"),
        ("/writing/routing", "/pages/writing-routing.js"),
    ] {
        assert!(
            json.contains(&format!("\"url\":\"{url}\"")),
            "{url} is missing from the manifest:\n{json}"
        );
        assert!(json.contains(&format!("\"module\":\"{module}\"")), "{json}");
    }
    assert!(json.contains("\"notFound\":\"/404\""), "{json}");
    // Double quotes, because the manifest is read by whatever serves the
    // site rather than by a JavaScript module.
    assert!(!json.contains('\''), "a manifest is JSON:\n{json}");
}

/// Every document must be a document: it renders in the embedded engine,
/// the way `examples/disclosure.zd` does.
#[test]
fn every_document_renders_in_the_embedded_engine() {
    let site = site_example();
    const DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
serialize($host)
"#;
    let expected = [
        ("/", "A site in one file"),
        ("/writing", "Routing without a page"),
        (
            "/writing/routing",
            "A route is a choice plus a bijection onto URLs.",
        ),
        (
            "/writing/folding",
            "An immutable signal has one value per document.",
        ),
        ("/404", "That URL is not part of this site."),
    ];
    for (url, needle) in expected {
        let mut context = context(false);
        let rendered = run(&mut context, &program(&site, url), DRIVER);
        assert!(
            rendered.contains(needle),
            "{url} did not render `{needle}`:\n{rendered}"
        );
    }
}

/// The links a document renders are the URLs the manifest serves. A link
/// to a URL nothing answers is the failure routing exists to make
/// impossible, so it is asserted rather than assumed.
#[test]
fn every_href_a_document_renders_is_a_url_the_site_serves() {
    let site = site_example();
    const DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
walk($host).filter((n) => n.tagName === 'a').map((n) => n.attributes.href).join(' ')
"#;
    let urls: Vec<&str> = site.pages.iter().map(|page| page.url.as_str()).collect();
    let mut checked = 0;
    for page in &site.pages {
        let mut context = context(false);
        let hrefs = run(&mut context, &program(&site, &page.url), DRIVER);
        for href in hrefs.split_whitespace() {
            checked += 1;
            assert!(
                urls.contains(&href),
                "{} links to {href}, which this site does not serve; it serves {urls:?}",
                page.url
            );
        }
    }
    // The example's nav links every page to every other, so a run that
    // found no anchors at all rendered nothing rather than rendering a
    // site with no bad links.
    assert!(
        checked >= site.pages.len(),
        "only {checked} anchors were rendered across {} documents",
        site.pages.len()
    );
}

/// An unrouted program is one document at `/`, which is what it has
/// always been. There is no second code path for a routed program to be
/// wrong in.
#[test]
fn an_unrouted_program_is_still_one_page() {
    let site = build(&support::repository_path("examples/counter.zd"));
    assert_eq!(site.pages.len(), 1);
    assert_eq!(site.pages[0].url, "/");
    assert!(site.pages[0]
        .client_js
        .contains("import { derived, signal }"));
}

// --- the accessibility default the address fold pays for (#142) -----------
//
// `aria-current="page"` marks the link to the document a reader is already
// on. It is not spellable as an argument — §16.3.6 makes an argument name a
// UAX#31 identifier and there is no hyphen in one — so it can only be a
// default the emitter adds, and it can only be a *compile-time* default in
// a compiler that knows both a document's URL and every link's destination
// while it emits. Both facts come from the address fold.

/// The nav link to this document is marked, and its siblings are not.
#[test]
fn a_link_to_the_document_it_sits_in_is_marked_as_the_current_page() {
    let site = site_example();
    let home = &page(&site, "/").client_js;
    let writing = &page(&site, "/writing").client_js;

    assert_eq!(
        marked(home),
        vec!["/"],
        "exactly the home page's link to itself is marked:\n{home}"
    );
    assert_eq!(
        marked(writing),
        vec!["/writing"],
        "exactly the writing page's link to itself is marked:\n{writing}"
    );
    // Both pages carry both links, or the equalities above would hold for
    // a page that had lost one.
    for (page, source) in [("/", home), ("/writing", writing)] {
        assert_eq!(
            anchors(source).len(),
            2,
            "{page} is supposed to carry a two-link nav:\n{source}"
        );
    }
}

/// Every `<a …>` start tag in an emitted module's templates.
fn anchors(source: &str) -> Vec<&str> {
    source
        .match_indices("<a ")
        .map(|(i, _)| &source[i..i + source[i..].find('>').expect("an unterminated tag")])
        .collect()
}

/// The `href` of every anchor marked as the current page.
fn marked(source: &str) -> Vec<&str> {
    anchors(source)
        .into_iter()
        .filter(|tag| tag.contains("aria-current=\"page\""))
        .map(|tag| {
            let rest = &tag[tag.find("href=\"").expect("an anchor with no href") + 6..];
            &rest[..rest.find('"').expect("an unterminated href")]
        })
        .collect()
}

/// A document that is not any nav destination marks nothing.
///
/// Without this the test above would also pass if the attribute were put on
/// every link, which is worse than putting it on none: it would tell a
/// screen reader that every destination is the current one.
#[test]
fn a_document_no_link_points_at_marks_no_link() {
    let site = site_example();
    let post = &page(&site, "/writing/routing").client_js;
    assert!(
        !post.contains("aria-current"),
        "a post page's nav links point elsewhere, so none of them is current:\n{post}"
    );
    // And it does have links, or the assertion above is vacuous.
    assert!(
        post.matches("<a ").count() >= 2,
        "the post page is supposed to carry a nav:\n{post}"
    );
}

// --- the shared stylesheet (#136) ----------------------------------------
//
// Splitting the *modules* per route was never in doubt — the tests above
// are it. What had never been measured was what the split leaves behind,
// and the answer was `base.css`: 3,641 bytes, identical for every document
// of a site, inlined into every one of them. `site.zd` wrote it five times
// and a ten-route site would write it eleven. That is 61% of everything a
// ten-route build writes, and it is the "ten copies of the prelude" §14A
// says a routed program must not ship.
//
// So the base sheet is a file the documents share, exactly as the runtime
// modules are, and each document links it. What stays per page is the
// generated classes, which really are per page.

/// **The measurement, as a gate.** No page carries the base stylesheet.
///
/// Stated as "every page's sheet together is smaller than one copy of the
/// base" rather than as a substring check, because that is the property
/// worth keeping: a sheet that inlined the base again would pass a check
/// for one missing selector and fail this.
#[test]
fn no_page_of_a_routed_site_carries_the_base_stylesheet() {
    let site = site_example();
    let base = zdc_runtime::BASE_CSS.len();
    let total: usize = site.pages.iter().map(|page| page.styles_css.len()).sum();

    assert!(
        total < base,
        "the {} documents write {total} bytes of CSS between them, against a \
         {base} byte base stylesheet. Inlining it per page is what #136 \
         measured and removed: it made 79 bytes of generated rules cost 3,720 \
         to fetch, and it charged every page after the first for a file the \
         reader already had.",
        site.pages.len()
    );
    for page in &site.pages {
        assert!(
            !page.styles_css.contains(".zd-col"),
            "{}'s sheet carries the base classes:\n{}",
            page.url,
            page.styles_css
        );
    }
    // And the generated rules survived, or the saving above is the saving
    // of emitting nothing.
    assert!(
        site.pages
            .iter()
            .any(|page| page.styles_css.contains(".zd-s0")),
        "the generated classes must still be emitted per page"
    );
}

/// It is shipped, once, with the bytes it has.
///
/// The set is what every writer of a bundle copies — `zdc build`, `zdc
/// dev`, a deploy adapter — so naming it here is what makes the link in
/// the documents resolve rather than 404.
#[test]
fn the_base_stylesheet_is_shipped_once_beside_the_runtime() {
    let site = site_example();
    assert!(
        site.runtime.contains(zdc_codegen::BASE_STYLESHEET),
        "a routed site must ship the sheet its documents link: {:?}",
        site.runtime
    );

    let files = zdc_codegen::runtime_files(&site.runtime, zdc_codegen::Mode::Release);
    let written: Vec<&str> = files
        .iter()
        .filter(|(path, _)| *path == zdc_codegen::BASE_STYLESHEET)
        .map(|(_, source)| source.as_str())
        .collect();
    assert_eq!(
        written,
        vec![zdc_runtime::BASE_CSS],
        "the shipped sheet is the base sheet, once, unmodified — a release \
         build must not run a JavaScript comment stripper over CSS"
    );
}

/// The cascade the single file used to have, restated as document order.
///
/// This is the one thing the split could get wrong and still look right:
/// `base.css` gives `.zd-row` a `display: flex`, and a program that says
/// `display is "block"` compiles to a generated class of equal
/// specificity. Equal specificity means the later rule wins, so the base
/// sheet has to be linked *first* — reversing these two links would
/// silently stop every program's styling from taking effect.
#[test]
fn a_page_links_the_shared_base_sheet_before_its_own() {
    let site = site_example();
    let mut checked = 0;
    for page in &site.pages {
        let document = page
            .document_html
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no document", page.url));
        let base = document
            .find("href=\"/runtime/base.css\"")
            .unwrap_or_else(|| panic!("{} does not link the base sheet:\n{document}", page.url));
        // By prefix: the name carries a content hash (#137), and spelling
        // one here would be this test asserting against its own copy of the
        // hashing rule rather than against the cascade it is about.
        let own = document
            .find(&format!("href=\"/pages/{}.", page.slug))
            .unwrap_or_else(|| panic!("{} does not link its own sheet:\n{document}", page.url));
        assert!(
            base < own,
            "{} links its generated rules before the base sheet, which reverses \
             the cascade:\n{document}",
            page.url
        );
        checked += 1;
    }
    // The loop above is the whole test, so how much of it ran is part of
    // the claim: a site that emitted no documents would satisfy it over
    // nothing.
    assert_eq!(checked, site.pages.len());
    assert!(checked >= 4, "the routed example emits five documents");
}

/// **The single-document case is untouched.**
///
/// A program with one document has nothing to share the base sheet with,
/// so splitting would buy it a second request and no bytes. §14A's
/// null-program figures are quoted against the one file it has always
/// written, and `BENCHMARKS.md` prints its length; both would move if this
/// stopped holding.
#[test]
fn an_unrouted_program_still_writes_one_stylesheet_with_the_base_inside_it() {
    let site = build(&support::repository_path("examples/counter.zd"));
    assert_eq!(site.pages.len(), 1, "counter.zd is not routed");

    let sheet = &site.pages[0].styles_css;
    assert!(
        sheet.starts_with(zdc_runtime::BASE_CSS),
        "an unrouted program's sheet still opens with the base:\n{sheet}"
    );
    assert!(
        !site.runtime.contains(zdc_codegen::BASE_STYLESHEET),
        "and it ships no second copy beside the runtime: {:?}",
        site.runtime
    );

    let document = site.pages[0]
        .document_html
        .as_ref()
        .expect("counter.zd renders a page");
    // By prefix, for the reason the routed case gives: the name carries a
    // content hash (#137), so `./styles.css` became `./styles.<hash>.css`.
    assert!(
        document.contains("href=\"./styles."),
        "the document links the one sheet it has:\n{document}"
    );
    assert!(
        !document.contains("base.css"),
        "and links no other:\n{document}"
    );
}

/// **A routed build names no entry, because it has none.**
///
/// The manifest said `client.js` for every build, and a routed one does
/// not write that file: its entries are `pages/<slug>.js`, one per URL,
/// and `routes.json` names them. A consumer following the manifest found
/// out by fetching a 404 (#369).
///
/// `null` rather than an absent key. A missing field reads as "this
/// compiler is too old to say", which is a different claim from "there is
/// no entry" and one a reader cannot tell apart from it.
#[test]
fn a_routed_manifest_names_no_entry_and_an_unrouted_one_does() {
    let site = site_example();
    assert!(
        site.manifest_json.contains(r#""entry":null"#),
        "a routed build has no single entry to name:\n{}",
        site.manifest_json
    );

    // The other half, so this cannot pass by the field having vanished.
    let page = build(&support::repository_path("examples/page.zd"));
    assert!(
        page.manifest_json.contains(r#""entry":"client.js""#),
        "an unrouted build writes `client.js` and should still say so:\n{}",
        page.manifest_json
    );
}

/// **The program's own code, written once.**
///
/// §14A's argument is bundle size, and the per-page split won half of it:
/// the runtime is shared and the program was not. Every route of a site
/// whose shell can reach a definition carries that definition, so the
/// portfolio shipped the same 427 KB thirty-three times and its 404 page
/// was 453 KB of JavaScript.
#[test]
fn a_definition_two_pages_reach_is_emitted_once_and_imported() {
    let site = site_example();
    let chunk = site
        .program_chunk
        .as_ref()
        .expect("a site whose pages share a definition emits a chunk");

    let shared: Vec<&PageBundle> = site
        .pages
        .iter()
        .filter(|page| page.client_js.contains(&chunk.name))
        .collect();
    assert!(
        shared.len() > 1,
        "a chunk exists because more than one page reads from it"
    );
    for page in &shared {
        assert!(
            page.client_js.contains(&format!("from './{}'", chunk.name)),
            "a page names the chunk beside itself, not at the site root — \
             both sit in `pages/`:\n{}",
            page.client_js
                .lines()
                .take(6)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    // The definition is in the chunk and in none of the pages that read
    // it. Emitting it in both would be the duplication this removes,
    // wearing an import.
    let example = "function titleOf(";
    assert!(
        chunk.client_js.contains(example),
        "the chunk holds what the pages share:\n{}",
        chunk.client_js
    );
    for page in &shared {
        assert!(
            !page.client_js.contains(example),
            "and a page that imports it does not also carry it:\n{}",
            page.slug
        );
    }
}

/// **What is imported is copied — the chunk included.**
///
/// `site.runtime` is a union over the documents and it is what every
/// writer of a bundle copies. Moving a definition into the chunk moves
/// its runtime import with it, so a module the chunk reaches and no page
/// reaches any more would be copied for nobody: the chunk would import a
/// file that is not there, and every page importing the chunk would fail
/// to load with it while `zdc build` reported nothing.
///
/// Asked of every emitted module rather than of the chunk alone. The
/// chunk of *this* program imports no runtime — it holds two pure text
/// functions — so a check that looked only there would pass without
/// looking at anything, which is what the first version of this test did
/// until its own guard said so. The pages always import runtime, so the
/// walk has something to walk, and the chunk is included wherever it has
/// imports of its own.
#[test]
fn the_runtime_every_emitted_module_imports_is_in_the_set_the_writer_copies() {
    let site = site_example();
    let chunk = site
        .program_chunk
        .as_ref()
        .map(|chunk| chunk.client_js.as_str());
    let modules = site
        .pages
        .iter()
        .map(|page| page.client_js.as_str())
        .chain(chunk);

    let mut checked = 0;
    for source in modules {
        for line in source.lines() {
            let Some(rest) = line.strip_prefix("import ") else {
                continue;
            };
            let Some(from) = rest.split(" from ").nth(1) else {
                continue;
            };
            let specifier = from.trim().trim_matches(|c| c == '\'' || c == ';');
            let Some(module) = specifier.strip_prefix("../runtime/") else {
                continue;
            };
            checked += 1;
            assert!(
                site.runtime.iter().any(|named| named.ends_with(module)),
                "`{module}` is imported and nothing would copy it: {:?}",
                site.runtime
            );
        }
    }
    assert!(
        checked > 0,
        "no emitted module imports the runtime, so this checked nothing"
    );
}
