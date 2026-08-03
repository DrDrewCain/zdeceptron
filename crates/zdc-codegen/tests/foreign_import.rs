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
