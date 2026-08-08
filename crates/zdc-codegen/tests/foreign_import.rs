//! What a `foreign` outside the `zd:` layer emits, and what it refuses.
//!
//! Spec §14E.1. The export reaches the generated `import` clause as
//! *syntax*, so no escaping makes an arbitrary string safe there and the
//! only available answer is to refuse the literal. That refusal is made at
//! parse time and asserted again here, at the emission site, because the
//! two guard different things: the parser guards one construct's syntax,
//! and this guards the position the name is written into.

mod support;

use support::compile_source;

/// §14E.2 links a foreign into whichever bundles actually call it, so the
/// import is a consequence of a call rather than of a declaration.
#[test]
fn a_called_foreign_is_imported_by_the_bundle_that_calls_it() {
    let bundle = compile_source(
        "foreign parse is anywhere\n\
         \x20   from \"marked\" as \"parse\"\n\
         \x20   takes source is Text\n\
         \x20   gives Text\n\
         state body is client Text starting \"hi\"\n\
         state out is client Text from parse with source is body\n\
         view\n\
         \x20   Column\n\
         \x20       Text out\n",
    );

    assert!(
        bundle.client_js.contains("from 'marked'"),
        "the module specifier is a string literal owning its own quotes:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("import { parse as"),
        "the export is written into the import clause as a bare name:\n{}",
        bundle.client_js
    );
}

/// §14E.2 links a foreign into whichever bundles call it, and a server
/// endpoint is one of those bundles (#223).
///
/// The server emitter wrote an unconditional `// No imports.` header, so a
/// `foreign` reached from a `server` signal was *called* and never
/// imported — `ReferenceError` on the first request, which is the same
/// failure the intrinsics preamble already exists to prevent for prelude
/// primitives.
#[test]
fn a_foreign_called_from_a_server_signal_is_imported_by_the_endpoint() {
    let bundle = compile_source(
        "foreign readAt is server\n\
         \x20   from \"./io.js\" as \"readAt\"\n\
         \x20   takes path is Text\n\
         \x20   gives Text\n\
         state contents is server Text from readAt with path is \"in.txt\"\n\
         view\n\
         \x20   Column\n\
         \x20       when contents\n\
         \x20           Loading           show Text \"…\"\n\
         \x20           Failed with error show Text error.message\n\
         \x20           Ready with body   show Text body\n",
    );

    let endpoint = bundle
        .functions
        .iter()
        .find(|f| f.name == "contents")
        .expect("the server signal emits an endpoint");

    assert!(
        endpoint.source.contains("import { readAt as"),
        "the endpoint calls `readAt`, so it has to import it:\n{}",
        endpoint.source
    );
    assert!(
        endpoint.source.contains("from './io.js'"),
        "the module specifier is a string literal owning its own quotes:\n{}",
        endpoint.source
    );
    assert!(
        !endpoint.source.contains("No imports"),
        "the header may not claim there are none when there are:\n{}",
        endpoint.source
    );
}

/// The claim in that header is still true for the ordinary case, and it is
/// worth keeping true: an endpoint reaching nothing outside `$env` and
/// `$store` should say so rather than carry an empty import section.
#[test]
fn an_endpoint_that_calls_no_foreign_still_says_it_imports_nothing() {
    let bundle = compile_source(
        "state hits is durable Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               add 1 to hits\n",
    );

    let endpoint = bundle
        .functions
        .first()
        .expect("a durable signal emits an endpoint");
    assert!(
        endpoint.source.contains("No imports"),
        "nothing was reached, so the header stands:\n{}",
        endpoint.source
    );
}

/// An emitted import has to point at a file the bundle contains (#223).
///
/// Both halves wrote an import and shipped nothing: `client.js` imported
/// `./gauge.js` from a bundle that held no `gauge.js`. The emitter cannot
/// copy files — `assets.rs` is the one part of this crate that touches the
/// filesystem, and `compile` takes its result as data — so the bundle
/// reports what has to be shipped and where, and the CLI does the copying.
#[test]
fn a_linked_foreign_reports_the_module_the_bundle_must_ship() {
    let bundle = compile_source(
        "foreign draw is client\n\
         \x20   from \"./draw.js\" as \"mount\"\n\
         \x20   takes level is Whole\n\
         \x20   gives Text\n\
         state n is client Whole starting 1\n\
         state out is client Text from draw with level is n\n\
         view\n\
         \x20   Column\n\
         \x20       Text out\n",
    );

    let shipped: Vec<(&str, &str)> = bundle
        .linked_modules
        .iter()
        .map(|m| (m.specifier.as_str(), m.destination.as_str()))
        .collect();

    assert_eq!(
        shipped,
        [("./draw.js", "draw.js")],
        "the client imports `./draw.js` beside `client.js`, so that is where it goes"
    );
}

/// A server endpoint lives in `functions/`, so a module it imports as
/// `./io.js` resolves to `functions/io.js` and has to be shipped there.
#[test]
fn a_server_foreign_ships_beside_the_endpoint_that_imports_it() {
    let bundle = compile_source(
        "foreign readAt is server\n\
         \x20   from \"./io.js\" as \"readAt\"\n\
         \x20   takes path is Text\n\
         \x20   gives Text\n\
         state contents is server Text from readAt with path is \"in.txt\"\n\
         view\n\
         \x20   Column\n\
         \x20       when contents\n\
         \x20           Loading           show Text \"…\"\n\
         \x20           Failed with error show Text error.message\n\
         \x20           Ready with body   show Text body\n",
    );

    let shipped: Vec<&str> = bundle
        .linked_modules
        .iter()
        .map(|m| m.destination.as_str())
        .collect();

    assert_eq!(shipped, ["functions/io.js"]);
}

/// A bare specifier is a package the target resolves, not a file this
/// build owns, so nothing is copied for it.
#[test]
fn a_package_specifier_is_imported_and_not_shipped() {
    let bundle = compile_source(
        "foreign parse is anywhere\n\
         \x20   from \"marked\" as \"parse\"\n\
         \x20   takes source is Text\n\
         \x20   gives Text\n\
         state body is client Text starting \"hi\"\n\
         state out is client Text from parse with source is body\n\
         view\n\
         \x20   Column\n\
         \x20       Text out\n",
    );

    assert!(
        bundle.client_js.contains("from 'marked'"),
        "the import is still written"
    );
    assert!(
        bundle.linked_modules.is_empty(),
        "but `marked` is not a file this build can copy: {:?}",
        bundle.linked_modules
    );
}

/// A declaration nothing calls is not linked, which is §14A.1's dead-code
/// elimination applied to dependencies.
#[test]
fn a_foreign_nothing_calls_is_not_imported() {
    let bundle = compile_source(
        "foreign parse is anywhere\n\
         \x20   from \"marked\" as \"parse\"\n\
         \x20   takes source is Text\n\
         \x20   gives Text\n\
         state n is client Whole starting 1\n\
         view\n\
         \x20   Column\n\
         \x20       Text n\n",
    );

    assert!(
        !bundle.client_js.contains("marked"),
        "an uncalled foreign was linked anyway:\n{}",
        bundle.client_js
    );
}

/// The injection this whole construct is shaped against.
///
/// It is refused by the *parser*, which is upstream of every later pass,
/// so there is no HIR to lower and no bundle to inspect. That is the
/// assertion: the refusal is not something emission does carefully, it is
/// something emission never gets the chance to do wrong.
#[test]
fn an_export_that_closes_the_import_clause_never_reaches_emission() {
    let error = zdc_parser::parse(
        "foreign parse is anywhere\n\
         \x20   from \"marked\" as \"mount } from 'evil'; //\"\n\
         \x20   takes source is Text\n\
         \x20   gives Text\n\
         view\n\
         \x20   Column\n",
    )
    .expect_err("an export that is not an identifier is refused");

    assert!(
        error
            .message
            .contains("not a name a JavaScript module can export"),
        "got {}",
        error.message
    );
}

/// A module specifier is a string literal, so escaping makes it
/// well-formed — and well-formed is not safe. A remote specifier needs no
/// injection at all to put a third party's code in the bundle with this
/// page's origin, so it is refused at resolution, before emission.
#[test]
fn a_foreign_from_a_remote_origin_never_reaches_emission() {
    let program = zdc_parser::parse(
        "foreign parse is anywhere\n\
         \x20   from \"https://evil.example/x.js\" as \"parse\"\n\
         \x20   takes source is Text\n\
         \x20   gives Text\n\
         view\n\
         \x20   Column\n",
    )
    .expect("a remote specifier is well-formed syntax; it is resolution that refuses it");

    let errors = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect_err("a remote specifier is refused");
    assert!(
        errors.iter().any(|e| e.message.contains("imports from")),
        "got {:?}",
        errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}
