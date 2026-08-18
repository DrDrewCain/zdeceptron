//! Content-hashed names, against the emitted bytes — #137.
//!
//! The bug this file exists to catch has one shape and it is quiet: the
//! document links a file the build did not write. Nothing refuses, nothing
//! warns, the page renders unstyled, and the only evidence is a 404 in a
//! network panel nobody has open. So every assertion below reads the name
//! out of the emitted document and compares it against what the bundle
//! says it wrote — never against a string spelled here, because a test
//! holding its own copy of the rule is a test that agrees with itself.
//!
//! The other half is the decision: **the ES module graph is not hashed.**
//! `crates/zdc-codegen/src/cache.rs` argues why — its leaves are
//! hand-written modules shipped byte for byte, whose `import './signal.js'`
//! this compiler never parses — and the tests at the bottom hold the line,
//! so that hashing `client.js` becomes a deliberate change rather than a
//! plausible-looking one.

mod support;

use support::{compile_example, compile_source, page, repository_path};
use zdc_codegen::{compile_site, Options, SiteBundle};

/// Every checked-in example that emits a page. Listed rather than globbed,
/// like `csp.rs`'s: a glob that stopped matching would leave this file
/// passing over nothing.
const EXAMPLES: [&str; 5] = [
    "examples/hello.zd",
    "examples/counter.zd",
    "examples/disclosure.zd",
    "examples/gauge.zd",
    "examples/todo.zd",
];

/// `examples/site.zd`, through the module loader and the build root — the
/// same pipeline `zdc build` runs, because a routed program enumerates its
/// URLs from `static` state and a shortcut here would emit against values
/// nothing computed.
fn routed_site() -> SiteBundle {
    let path = repository_path("examples/site.zd");
    let linked = zdc_resolve::load(&path)
        .unwrap_or_else(|failure| panic!("load: {}", failure.errors[0].message));
    let hir = zdc_resolve::Resolver::linked(&linked)
        .resolve()
        .unwrap_or_else(|errors| panic!("resolve: {}", errors[0].message));
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
    compile_site(&inputs, &options).unwrap_or_else(|errors| panic!("emit: {}", errors[0].message))
}

/// The href of the first stylesheet a document links.
fn linked_stylesheet(document: &str) -> String {
    const MARKER: &str = "<link rel=\"stylesheet\" href=\"";
    let at = document
        .find(MARKER)
        .unwrap_or_else(|| panic!("no stylesheet is linked:\n{document}"))
        + MARKER.len();
    let end = document[at..].find('"').expect("an href ends") + at;
    document[at..end].to_string()
}

#[test]
fn the_document_links_the_stylesheet_the_bundle_says_it_wrote() {
    let mut checked = 0;
    for example in EXAMPLES {
        let bundle = compile_example(example);
        let linked = linked_stylesheet(page(&bundle));
        assert_eq!(
            linked,
            format!("./{}", bundle.styles_path),
            "{example} links a stylesheet the build does not write"
        );
        assert_ne!(
            bundle.styles_path, "styles.css",
            "{example}: the name must carry a content hash"
        );
        assert!(
            bundle.styles_path.starts_with("styles.") && bundle.styles_path.ends_with(".css"),
            "{example}: {}",
            bundle.styles_path
        );
        assert_eq!(
            bundle.immutable,
            vec![bundle.styles_path.clone()],
            "{example}: only a hashed name may be cached for a year"
        );
        checked += 1;
    }
    assert_eq!(checked, EXAMPLES.len(), "the list stopped being iterated");
}

/// Both directions, on the artifact rather than on the hash function.
/// Same program, same URL — or every deploy invalidates every cache and no
/// build is reproducible. Different stylesheet, different URL — or a
/// browser keeps serving bytes it was told never to check again.
#[test]
fn the_name_is_stable_for_identical_output_and_changes_when_it_changes() {
    const NARROW: &str = "view\n    Column padding is 8\n        Text \"x\"\n";
    const WIDE: &str = "view\n    Column padding is 12\n        Text \"x\"\n";

    let once = compile_source(NARROW);
    let again = compile_source(NARROW);
    assert_eq!(
        once.styles_path, again.styles_path,
        "two builds of one program must agree"
    );
    assert_eq!(once.styles_css, again.styles_css);

    let other = compile_source(WIDE);
    assert_ne!(
        once.styles_css, other.styles_css,
        "the fixture is wrong: these must differ in the stylesheet"
    );
    assert_ne!(
        once.styles_path, other.styles_path,
        "a changed stylesheet must be a changed URL"
    );
}

/// A routed program: one stylesheet per document, each linked from its own
/// page and named in `routes.json`, which is what a host reads to answer a
/// request without running the compiler.
#[test]
fn every_routed_document_links_and_records_its_own_stylesheet() {
    let site = routed_site();
    assert!(site.pages.len() > 1, "site.zd is supposed to be routed");

    for page in &site.pages {
        let document = page
            .document_html
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no document", page.url));
        assert_eq!(
            linked_stylesheet(document),
            format!("/{}", page.styles_path),
            "{} links a stylesheet the build does not write",
            page.url
        );
        assert!(
            page.styles_path
                .starts_with(&format!("pages/{}.", page.slug)),
            "{}: a page's stylesheet keeps its slug as well as its hash: {}",
            page.url,
            page.styles_path
        );
        assert!(
            site.routes_json
                .contains(&format!("\"styles\":\"/{}\"", page.styles_path)),
            "{} is not in the route table under the name it was written:\n{}",
            page.url,
            site.routes_json
        );
        assert!(
            site.immutable.contains(&page.styles_path),
            "{} may not be cached, and it carries a hash",
            page.url
        );
    }
    assert_eq!(site.immutable.len(), site.pages.len());
}

/// The decision, held. A hash on a file reached by an `import` would have
/// to be matched by a rewrite of the specifier that reaches it, and the
/// specifiers inside `runtime/*.js` live in files this compiler ships
/// unmodified. So the graph is left whole — entry included — and this test
/// is what makes changing that a decision rather than an accident.
#[test]
fn nothing_reached_by_an_import_carries_a_hash() {
    let bundle = compile_example("examples/counter.zd");

    let document = page(&bundle);
    assert!(
        document.contains("<script type=\"module\" src=\"./boot.js\"></script>"),
        "the page's module is `boot.js`, unhashed:\n{document}"
    );
    let boot = bundle.boot_js.as_deref().expect("a boot module");
    assert!(
        boot.contains("from './client.js'"),
        "the boot module imports `client.js`, unhashed:\n{boot}"
    );
    assert!(
        bundle.client_js.contains("from './runtime/"),
        "the runtime is imported from a directory with no hash in it:\n{}",
        bundle.client_js
    );
    for module in &bundle.runtime {
        assert!(
            module.starts_with("runtime/") && module.ends_with(".js"),
            "{module} carries a hash, and nothing rewrites what imports it"
        );
    }
    assert!(
        !bundle.immutable.iter().any(|path| path.ends_with(".js")),
        "no JavaScript may be immutable while its name cannot change: {:?}",
        bundle.immutable
    );
}
