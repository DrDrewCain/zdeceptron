//! What a `foreign` outside the `zd:` layer emits, and what it refuses.
//!
//! Spec §14E.1. The export reaches the generated `import` clause as
//! *syntax*, so no escaping makes an arbitrary string safe there and the
//! only available answer is to refuse the literal. That refusal is made at
//! parse time and asserted again here, at the emission site, because the
//! two guard different things: the parser guards one construct's syntax,
//! and this guards the position the name is written into.

mod support;

use support::{compile_source, Project};

/// The mapping every test below that imports `marked` compiles against.
///
/// A bare specifier resolves through the project's `[packages]` table and
/// nowhere else (#238), so there is no such thing as compiling one without
/// a project to read it from.
const MARKED: &str = "marked = \"https://esm.sh/marked@15.0.7\"\n";

/// §14E.2 links a foreign into whichever bundles actually call it, so the
/// import is a consequence of a call rather than of a declaration.
#[test]
fn a_called_foreign_is_imported_by_the_bundle_that_calls_it() {
    let bundle = Project::build(
        "called",
        MARKED,
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

/// A bare specifier mapped to a URL names a module the browser fetches,
/// not a file this build owns, so nothing is copied for it.
#[test]
fn a_package_specifier_is_imported_and_not_shipped() {
    let bundle = Project::build(
        "not-shipped",
        MARKED,
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
    let bundle = Project::build(
        "uncalled",
        MARKED,
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
    let page = bundle.index_html.expect("the program renders a page");
    assert!(
        !page.contains("importmap"),
        "the map carries the packages the bundle imports, and it imports none:\n{page}"
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

/// A URL specifier is emitted as written (#238).
///
/// It used to be refused, on the grounds that a remote origin runs with
/// this page's origin. That is true and it was not what the rule achieved:
/// the alternative to a refused URL is a two-line `.js` file importing the
/// same URL — the risk relocated to where the compiler cannot see it.
/// Written here, it is in the declaration, in the manifest, and pinnable
/// later.
#[test]
fn a_url_specifier_is_emitted_as_written() {
    let bundle = Project::build(
        "url",
        "",
        "foreign parse is anywhere\n\
         \x20   from \"https://esm.sh/marked@15.0.7\" as \"parse\"\n\
         \x20   takes source is Text\n\
         \x20   gives Text\n\
         state body is client Text starting \"hi\"\n\
         state out is client Text from parse with source is body\n\
         view\n\
         \x20   Column\n\
         \x20       Text out\n",
    );

    assert!(
        bundle
            .client_js
            .contains("from 'https://esm.sh/marked@15.0.7'"),
        "the specifier is the import:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.linked_modules.is_empty(),
        "a URL is fetched, not shipped: {:?}",
        bundle.linked_modules
    );
    let page = bundle.index_html.expect("the program renders a page");
    assert!(
        !page.contains("importmap"),
        "a URL resolves on its own, so there is nothing for a map to say:\n{page}"
    );
}

/// A deploy target and a reader both have to be able to enumerate what the
/// page fetches at load, and neither of them runs the compiler. Both
/// spellings reach the same list: the URL written in the declaration, and
/// the URL a bare specifier was mapped to.
#[test]
fn every_remote_origin_the_bundle_imports_is_in_the_manifest() {
    let bundle = Project::build(
        "origins",
        MARKED,
        "foreign parse is anywhere\n\
         \x20   from \"marked\" as \"parse\"\n\
         \x20   takes source is Text\n\
         \x20   gives Text\n\
         foreign slug is anywhere\n\
         \x20   from \"https://cdn.example.test/slugify@1.6.6\" as \"default\"\n\
         \x20   takes value is Text\n\
         \x20   gives Text\n\
         state body is client Text starting \"hi\"\n\
         state out is client Text from parse with source is body\n\
         state tag is client Text from slug with value is body\n\
         view\n\
         \x20   Column\n\
         \x20       Text out\n\
         \x20       Text tag\n",
    );

    assert!(
        bundle
            .manifest_json
            .contains("\"origins\":[\"https://cdn.example.test\",\"https://esm.sh\"]"),
        "both origins, sorted, and the origin rather than the whole URL:\n{}",
        bundle.manifest_json
    );
}

/// The map is one per document, carries only the packages the bundle
/// actually imports, and precedes the module script — which is document
/// order the browser requires, not a preference. A map that arrives after
/// the first module load is ignored, and the page fails exactly as it did
/// with no map at all.
#[test]
fn the_import_map_precedes_the_module_script() {
    let bundle = Project::build(
        "map-order",
        MARKED,
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

    let page = bundle.index_html.expect("the program renders a page");
    let map = page
        .find("<script type=\"importmap\">")
        .unwrap_or_else(|| panic!("no import map in:\n{page}"));
    // The opening tag only, not a closing `>`: the policy forbids an
    // inline script, so the module load is now
    // `<script type="module" src="./boot.js">`. What this test is about is
    // the *order* of the two tags, and that is unchanged.
    let module = page
        .find("<script type=\"module\"")
        .unwrap_or_else(|| panic!("no module script in:\n{page}"));
    assert!(
        map < module,
        "the map has to be parsed before the first module load:\n{page}"
    );
    assert!(
        page.contains("</head>") && map < page.find("</head>").expect("a head"),
        "the map belongs in the head:\n{page}"
    );
    assert!(
        page.contains("{\"imports\":{\"marked\":\"https://esm.sh/marked@15.0.7\"}}"),
        "the map says what the project said, and nothing else:\n{page}"
    );
}

/// A vendored copy is expressible, and it goes through the `linked_module`
/// machinery #223 already built rather than through a second filesystem
/// path: the mapping names a file this build owns, so the build ships it.
#[test]
fn a_relative_mapping_target_is_shipped_with_the_bundle() {
    let bundle = Project::build(
        "vendored",
        "marked = \"./vendor/marked.js\"\n",
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

    let shipped: Vec<(&str, &str)> = bundle
        .linked_modules
        .iter()
        .map(|m| (m.specifier.as_str(), m.destination.as_str()))
        .collect();
    assert_eq!(
        shipped,
        [("./vendor/marked.js", "vendor/marked.js")],
        "the mapping names a file, so the file is shipped"
    );
    let page = bundle.index_html.expect("the program renders a page");
    assert!(
        page.contains("{\"imports\":{\"marked\":\"./vendor/marked.js\"}}"),
        "and the map points the browser at where it landed:\n{page}"
    );
    assert!(
        bundle.manifest_json.contains("\"origins\":[]"),
        "a vendored copy is fetched from nowhere remote:\n{}",
        bundle.manifest_json
    );
}

/// An endpoint is a standalone file on a server with no document, so there
/// is no import map for it to consult. It gets the target substituted into
/// the import instead — which is the same resolution, reached the only way
/// available on that side of the wire.
#[test]
fn an_endpoint_imports_the_target_a_bare_specifier_was_mapped_to() {
    let bundle = Project::build(
        "endpoint",
        MARKED,
        "foreign parse is server\n\
         \x20   from \"marked\" as \"parse\"\n\
         \x20   takes source is Text\n\
         \x20   gives Text\n\
         state out is server Text from parse with source is \"hi\"\n\
         view\n\
         \x20   Column\n\
         \x20       when out\n\
         \x20           Loading           show Text \"…\"\n\
         \x20           Failed with error show Text error.message\n\
         \x20           Ready with body   show Text body\n",
    );

    let endpoint = bundle
        .functions
        .iter()
        .find(|f| f.name == "out")
        .expect("the server signal emits an endpoint");
    assert!(
        endpoint
            .source
            .contains("from 'https://esm.sh/marked@15.0.7'"),
        "no document, no map, so the target is the specifier:\n{}",
        endpoint.source
    );
    assert!(
        bundle
            .manifest_json
            .contains("\"origins\":[\"https://esm.sh\"]"),
        "what the server fetches is still what this bundle imports:\n{}",
        bundle.manifest_json
    );
}

/// `gives new Handle` — the export is a class, so the call constructs.
///
/// The whole of issue #271's first missing piece: three.js exports classes
/// and `Class constructor WebGLRenderer cannot be invoked without 'new'`
/// is what a program got instead of a scene. The import is unchanged —
/// the same `import { … } from …` an ordinary foreign emits — and only
/// the application site differs.
#[test]
fn a_constructing_foreign_emits_new() {
    let bundle = compile_source(
        "foreign vector is client\n\
         \x20   from \"./three.module.js\" as \"Vector3\"\n\
         \x20   takes x is Decimal, y is Decimal, z is Decimal\n\
         \x20   gives new Handle\n\
         foreign lengthOf is client\n\
         \x20   from \"./three.module.js\" as \"Vector3\"\n\
         \x20   takes v is Handle\n\
         \x20   gives Decimal\n\
         state size is client Decimal from lengthOf with v is (vector with x is 3, y is 4, z is 0)\n\
         view\n\
         \x20   Column\n\
         \x20       Text size\n",
    );

    assert!(
        bundle.client_js.contains("import { Vector3 as"),
        "a constructing foreign is imported exactly as any other is:\n{}",
        bundle.client_js
    );
    assert!(
        bundle
            .client_js
            .lines()
            .any(|line| line.contains("new ") && line.contains("(3, 4, 0)")),
        "the call has to be a construction, not an invocation:\n{}",
        bundle.client_js
    );
}

/// A `foreign` that hands back a value is still called, so `new` is a
/// property of the declaration and not of the type.
#[test]
fn an_ordinary_foreign_is_still_called() {
    // A path rather than the bare `marked` this was written with: #238
    // landed after it, and a bare specifier now has to be mapped in
    // `zd.toml`. What this test is about is the call, not the specifier.
    let bundle = compile_source(
        "foreign parse is anywhere\n\
         \x20   from \"./marked.js\" as \"parse\"\n\
         \x20   takes source is Text\n\
         \x20   gives Text\n\
         state out is client Text from parse with source is \"hi\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text out\n",
    );
    assert!(
        !bundle.client_js.contains("new "),
        "nothing here constructs:\n{}",
        bundle.client_js
    );
}

/// `on Handle as "add"` — the symbol is a method on the call's first
/// argument, and **nothing is imported**: a method comes with the object.
///
/// The second half of #271. `scene.add(mesh)` is what three.js is made of
/// and no ZDeceptron expression could say it.
#[test]
fn a_method_foreign_calls_its_first_argument_and_imports_nothing() {
    let bundle = compile_source(
        "foreign vector is client\n\
         \x20   from \"./three.module.js\" as \"Vector3\"\n\
         \x20   takes x is Decimal, y is Decimal, z is Decimal\n\
         \x20   gives new Handle\n\
         foreign plus is client\n\
         \x20   on Handle as \"add\"\n\
         \x20   takes target is Handle, other is Handle\n\
         \x20   gives Handle\n\
         foreign lengthOf is client\n\
         \x20   on Handle as \"length\"\n\
         \x20   takes of v is Handle\n\
         \x20   gives Decimal\n\
         state size is client Decimal from lengthOf of (plus with target is (vector with x is 1, y is 2, z is 2), other is (vector with x is 2, y is 4, z is 4))\n\
         view\n\
         \x20   Column\n\
         \x20       Text size\n",
    );

    assert!(
        bundle
            .client_js
            .contains(".add(new vector(2, 4, 4)).length()"),
        "the receiver comes first and the rest of the arguments follow it:\n{}",
        bundle.client_js
    );
    assert_eq!(
        bundle.client_js.matches("import {").count(),
        3,
        "three imports — the two runtime modules and `Vector3`. A method names no module, so \
         neither `add` nor `length` adds one:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("add as") && !bundle.client_js.contains("length as"),
        "a method is looked up at run time and is never imported:\n{}",
        bundle.client_js
    );
}

/// `gives nothing` and `do` — the second of the three things #276 named
/// as blocking stage 3.
///
/// `scene.add(mesh)` and `renderer.render(scene, camera)` both hand back
/// `undefined`, and before this there was no statement position a call
/// could be written in without its result going somewhere. The emission is
/// an expression statement: no `const`, no `return`.
#[test]
fn an_effect_is_emitted_as_an_expression_statement() {
    let bundle = compile_source(
        "foreign scene is client\n\
         \x20   from \"./three.module.js\" as \"Scene\"\n\
         \x20   gives new Handle\n\
         foreign mesh is client\n\
         \x20   from \"./three.module.js\" as \"Mesh\"\n\
         \x20   gives new Handle\n\
         foreign addTo is client\n\
         \x20   on Handle as \"add\"\n\
         \x20   takes parent is Handle, child is Handle\n\
         \x20   gives nothing\n\
         state n is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Button \"grow\"\n\
         \x20           on click\n\
         \x20               do addTo with parent is scene, child is mesh\n\
         \x20               add 1 to n\n\
         \x20       Text n\n",
    );

    assert!(
        bundle.client_js.contains("new scene().add(new mesh());"),
        "an effect is a bare call followed by a semicolon:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("= new scene().add"),
        "nothing names the result of a call that has none:\n{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("Mesh as mesh"),
        "a foreign named only by a `do` is still reachable, so the bundle imports it:\n{}",
        bundle.client_js
    );
}

/// `of Handle as "domElement"` — the symbol is a **property**, and the
/// emission is member access with no argument list at all.
///
/// The first of the three things #276 named as blocking stage 3.
/// `renderer.domElement` is the canvas a WebGL renderer made for itself,
/// and `renderer.domElement()` is a `TypeError` — so what this pins is the
/// absence of the parentheses, which is the whole difference between a
/// property and a method.
#[test]
fn a_property_foreign_reads_its_first_argument_and_imports_nothing() {
    let bundle = compile_source(
        "foreign renderer is client\n\
         \x20   from \"./three.module.js\" as \"WebGLRenderer\"\n\
         \x20   gives new Handle\n\
         foreign canvasOf is client\n\
         \x20   of Handle as \"domElement\"\n\
         \x20   takes of r is Handle\n\
         \x20   gives Handle\n\
         foreign widthOf is client\n\
         \x20   of Handle as \"width\"\n\
         \x20   takes of c is Handle\n\
         \x20   gives Whole\n\
         state size is client Whole from widthOf of (canvasOf of renderer)\n\
         view\n\
         \x20   Column\n\
         \x20       Text size\n",
    );

    assert!(
        bundle.client_js.contains("new renderer().domElement.width"),
        "a property is member access and nothing else:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("domElement()") && !bundle.client_js.contains("width()"),
        "a property is read, never called:\n{}",
        bundle.client_js
    );
    assert_eq!(
        bundle.client_js.matches("import {").count(),
        3,
        "three imports — the two runtime modules and `WebGLRenderer`. A property names no \
         module, so neither `domElement` nor `width` adds one:\n{}",
        bundle.client_js
    );
}

/// The receiver is the first argument and everything after it is inside
/// the call's own parentheses.
///
/// The receiver itself is emitted through `Expr::operand(MEMBER)`, so an
/// expression binding more loosely than a dot would be parenthesised.
/// Nothing binding that loosely can have type `Handle` today — only a
/// `foreign` call produces one, and a call binds as tightly as a dot — so
/// what this pins is the argument side, which a program can write freely.
#[test]
fn a_method_takes_its_receiver_first_and_its_arguments_after() {
    let bundle = compile_source(
        "foreign make is client\n\
         \x20   from \"./m.js\" as \"Box\"\n\
         \x20   takes n is Whole\n\
         \x20   gives new Handle\n\
         foreign pick is client\n\
         \x20   on Handle as \"pick\"\n\
         \x20   takes v is Handle, index is Whole\n\
         \x20   gives Whole\n\
         state n is client Whole from pick with v is (make with n is 1), index is 2 + 3\n\
         view\n\
         \x20   Column\n\
         \x20       Text n\n",
    );
    assert!(
        bundle.client_js.contains(".pick(2 + 3)"),
        "an argument after the receiver is inside the call's own parentheses:\n{}",
        bundle.client_js
    );
}
