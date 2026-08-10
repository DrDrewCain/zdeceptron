//! The Content-Security-Policy every emitted page carries — #146.
//!
//! §16.3.5's argument is that the compiler knows things about the emitted
//! program that an application generally cannot know about itself: that
//! `innerHTML` is assigned in exactly three named functions inside the
//! runtime, that generated code never spells the property, that no
//! `style` attribute is ever written, that a form has no `action`. A
//! policy is that knowledge stated to the browser, which then enforces it
//! at the point of use.
//!
//! **The failure mode this file exists to prevent is a policy the emitted
//! program violates.** A policy that blocks the program's own script is
//! worse than no policy: the page renders nothing, and it does so only in
//! a browser, which is the one place none of the other suites run. So
//! every assertion below is against the emitted bytes rather than against
//! the constant, and the ones that need a browser are named where they
//! cannot be made here.

mod support;

use support::{compile_example, repository_path};

use zdc_codegen::{compile_site, Options, SiteBundle, CONTENT_SECURITY_POLICY};

/// Every checked-in example that emits a page, so the claims below are
/// about the population and not about one program.
///
/// Listed rather than globbed: a glob that stopped matching would leave
/// this file passing over nothing, which is the failure the repository's
/// own vacuity gate exists for.
const EXAMPLES: [&str; 7] = [
    "examples/hello.zd",
    "examples/counter.zd",
    "examples/disclosure.zd",
    "examples/events.zd",
    "examples/gauge.zd",
    "examples/todo.zd",
    "examples/voting-board.zd",
];

/// The directive a policy names, or `None` when it does not name it.
fn directive<'a>(policy: &'a str, name: &str) -> Option<&'a str> {
    policy
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(name).filter(|rest| rest.starts_with(' ')))
        .map(str::trim)
}

#[test]
fn every_emitted_page_carries_the_policy() {
    let mut checked = 0;
    for example in EXAMPLES {
        let bundle = compile_example(example);
        let page = bundle
            .index_html
            .unwrap_or_else(|| panic!("{example} emits no page"));
        assert!(
            page.contains(&format!(
                "<meta http-equiv=\"Content-Security-Policy\" content=\"{CONTENT_SECURITY_POLICY}\">"
            )),
            "{example} carries no policy:\n{page}"
        );
        checked += 1;
    }
    assert_eq!(checked, EXAMPLES.len(), "the list stopped being iterated");
}

/// The claim `script-src 'self'` makes, checked against what is emitted.
///
/// This is the directive the whole policy turns on, and the one an
/// emitted page would violate first. It holds only while the document has
/// no inline script, so that is what is asserted — on the document, per
/// example, rather than on the constant.
#[test]
fn no_emitted_page_contains_an_inline_script() {
    assert_eq!(
        directive(CONTENT_SECURITY_POLICY, "script-src"),
        Some("'self'"),
        "the assertions below are about this exact directive"
    );
    let mut checked = 0;
    for example in EXAMPLES {
        let bundle = compile_example(example);
        let page = bundle.index_html.expect("a page");
        // Every `<script>` in the document is a `src`, and its content is
        // empty. An inline one is what the policy would block.
        for (index, after) in page.match_indices("<script").map(|(i, _)| (i, &page[i..])) {
            let element = after.split_once('>').expect("an unterminated script tag").0;
            assert!(
                element.contains(" src="),
                "{example} has an inline script at byte {index}:\n{page}"
            );
        }
        assert!(
            page.contains("<script type=\"module\" src="),
            "{example} does not load its module:\n{page}"
        );
        checked += 1;
    }
    assert_eq!(checked, EXAMPLES.len(), "the list stopped being iterated");
}

/// The page names a module the build actually writes.
///
/// The policy allows `'self'` and nothing else, so a page pointing at a
/// file the bundle does not contain is a page whose script never loads —
/// which under a strict policy looks exactly like a policy violation and
/// is not one.
#[test]
fn the_page_loads_a_module_the_bundle_writes() {
    let bundle = compile_example("examples/counter.zd");
    let page = bundle.index_html.expect("a page");
    assert!(
        page.contains("<script type=\"module\" src=\"./boot.js\"></script>"),
        "the page must load `boot.js`:\n{page}"
    );
    let boot = bundle.boot_js.expect("a boot module");
    assert!(
        boot.contains("import { main } from './client.js';"),
        "the boot module must import the bundle:\n{boot}"
    );
    assert!(
        boot.contains("main(document.getElementById('app'))"),
        "the boot module must mount:\n{boot}"
    );
    assert!(
        page.contains("<div id=\"app\"></div>"),
        "the mount point the boot module names must exist:\n{page}"
    );
}

/// `'unsafe-eval'` is absent, and the emitted bytes are why it can be.
///
/// Not a claim about the constant — that would be a tautology — but about
/// the JavaScript a build ships: nothing in it evaluates a string.
#[test]
fn nothing_a_bundle_ships_needs_unsafe_eval() {
    assert!(
        !CONTENT_SECURITY_POLICY.contains("unsafe-eval"),
        "the policy allows string evaluation"
    );
    let bundle = compile_example("examples/todo.zd");
    let mut scanned = 0;
    let mut sources = vec![("client.js".to_string(), bundle.client_js.clone())];
    for (name, source) in zdc_codegen::runtime_files(&bundle.runtime, zdc_codegen::Mode::Release) {
        sources.push((name.to_string(), source));
    }
    for function in &bundle.functions {
        sources.push((function.path.clone(), function.source.clone()));
    }
    assert!(
        sources.len() >= 3,
        "only {} files scanned; a `durable` bundle ships more",
        sources.len()
    );
    for (name, source) in &sources {
        for forbidden in ["eval(", "new Function(", "setTimeout('", "setInterval('"] {
            assert!(
                !source.contains(forbidden),
                "{name} uses `{forbidden}`, which `script-src 'self'` blocks"
            );
        }
        scanned += 1;
    }
    assert_eq!(scanned, sources.len());
}

/// `style-src 'self'` holds because no `style` attribute is ever emitted.
///
/// The emitter refuses a `style` argument and folds static declarations
/// into a generated class; a reactive one is `bindStyle`, which is CSSOM
/// and outside CSP's reach. This checks the first half against the bytes.
#[test]
fn no_emitted_markup_carries_a_style_attribute() {
    assert_eq!(
        directive(CONTENT_SECURITY_POLICY, "style-src"),
        Some("'self'"),
        "this test is about this exact directive"
    );
    let mut checked = 0;
    for example in EXAMPLES {
        let bundle = compile_example(example);
        for (what, source) in [
            ("index.html", bundle.index_html.clone().unwrap_or_default()),
            ("client.js", bundle.client_js.clone()),
        ] {
            assert!(
                !source.contains("style=\""),
                "{example}'s {what} writes a style attribute, which the policy blocks:\n{source}"
            );
            assert!(
                !source.contains("<style"),
                "{example}'s {what} writes a style element, which the policy blocks"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, EXAMPLES.len() * 2);
}

/// A styled program still puts its declarations in the stylesheet.
///
/// The counterpart to the test above: it would also pass if the emitter
/// had stopped emitting styles at all.
#[test]
fn a_styled_program_puts_its_declarations_in_the_stylesheet() {
    let bundle = compile_example("examples/todo.zd");
    let generated: Vec<&str> = bundle
        .styles_css
        .lines()
        .filter(|line| line.starts_with(".zd-s"))
        .collect();
    assert!(
        !generated.is_empty(),
        "todo.zd declares styles and none folded into a class:\n{}",
        bundle.styles_css
    );
    assert!(
        !bundle.client_js.contains("style="),
        "and they must not also be an attribute:\n{}",
        bundle.client_js
    );
}

/// The directives that are only enforceable as a header are not written.
///
/// `frame-ancestors`, `report-uri` and `sandbox` are ignored inside a
/// `<meta http-equiv>`. Emitting one would look like protection and be a
/// console warning, which is the specific kind of dishonesty a policy
/// must not have.
#[test]
fn the_policy_writes_no_directive_a_meta_tag_ignores() {
    for ignored in ["frame-ancestors", "report-uri", "report-to", "sandbox"] {
        assert!(
            !CONTENT_SECURITY_POLICY.contains(ignored),
            "`{ignored}` does nothing in a meta tag and must be a response header"
        );
    }
    // And the fallback really is refusal, or the directives above would be
    // the only thing standing between the page and anything at all.
    assert_eq!(
        directive(CONTENT_SECURITY_POLICY, "default-src"),
        Some("'none'")
    );
}

/// The URL-bearing directives are the compiler's own scheme allowlist.
///
/// `zdc_hir::url_is_safe` and `safeUrl` refuse everything outside
/// `URL_SCHEMES`, so the browser is being asked to enforce the same list a
/// second time. Deriving the assertion from that list means widening the
/// language's schemes without widening the policy fails here.
#[test]
fn the_fetch_directives_match_the_compilers_url_allowlist() {
    let fetching: Vec<&str> = zdc_codegen::URL_SCHEMES
        .iter()
        .copied()
        .filter(|scheme| *scheme == "http" || *scheme == "https")
        .collect();
    assert_eq!(
        fetching,
        vec!["http", "https"],
        "the schemes that fetch have changed; the policy has to change with them"
    );
    let mut checked = 0;
    for name in ["img-src", "font-src", "media-src", "frame-src"] {
        let value = directive(CONTENT_SECURITY_POLICY, name)
            .unwrap_or_else(|| panic!("`{name}` is missing, so `default-src 'none'` blocks it"));
        assert_eq!(value, "'self' http: https:", "`{name}` disagrees");
        checked += 1;
    }
    assert_eq!(checked, 4);
    // The three that are `'none'`, each because the vocabulary cannot
    // reach them at all.
    for name in ["object-src", "base-uri", "form-action"] {
        assert_eq!(
            directive(CONTENT_SECURITY_POLICY, name),
            Some("'none'"),
            "`{name}` must be refused outright"
        );
    }
}

/// The same pipeline `zdc build` runs, so a routed program is emitted the
/// way it ships rather than the way a shortcut would emit it.
fn site(relative: &str) -> SiteBundle {
    let path = repository_path(relative);
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
    // §17.4.8's build root, exactly as `zdc build` runs it: a routed
    // program enumerates its URLs from `static` state, so a test that
    // skipped this would emit against values nothing computed.
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

/// A routed program's documents carry it too, and each names its own
/// module.
#[test]
fn every_document_of_a_routed_program_carries_the_policy() {
    let site = site("examples/site.zd");
    assert!(site.pages.len() > 1, "site.zd is supposed to be routed");
    let mut checked = 0;
    for page in &site.pages {
        let document = page
            .document_html
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no document", page.url));
        assert!(
            document.contains("Content-Security-Policy"),
            "{} carries no policy:\n{document}",
            page.url
        );
        let boot = format!("/pages/{}.boot.js", page.slug);
        assert!(
            document.contains(&format!("src=\"{boot}\"")),
            "{} does not load its own module:\n{document}",
            page.url
        );
        let source = page
            .boot_js
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no boot module", page.url));
        assert!(
            source.contains(&format!("'/pages/{}.js'", page.slug)),
            "{}'s boot module imports the wrong page:\n{source}",
            page.url
        );
        checked += 1;
    }
    assert_eq!(checked, site.pages.len());
}
