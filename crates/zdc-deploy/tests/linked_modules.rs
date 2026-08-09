//! Where a `foreign`'s module lands in a deployment (#225).
//!
//! `zdc build` ships the modules an emitted `import` names; `zdc deploy`
//! did not, so a deployment carried imports pointing at files it did not
//! contain. The failure is the same one #223 fixed for the build, moved to
//! where it is hardest to read: on a platform, away from the machine that
//! generated the tree, in whatever words that platform has for a module it
//! cannot resolve.
//!
//! Nothing here writes a file. This crate reports what has to be copied and
//! where, exactly as `zdc-codegen` does, and `zdc-cli`'s `tests/deploy.rs`
//! asserts the copy actually happens on disk.

mod support;

use std::collections::BTreeSet;

use support::{compile_source, program};
use zdc_deploy::{generate, Options, Target};

/// A program whose browser half and whose endpoint each call a `foreign`
/// of their own. No example has this shape, and it is the shape that
/// matters: the two halves land in different directories.
const BOTH_HALVES: &str = concat!(
    "foreign draw is client\n",
    "    from \"./draw.js\" as \"mount\"\n",
    "    takes level is Whole\n",
    "    gives Text\n",
    "foreign readAt is server\n",
    "    from \"./io.js\" as \"readAt\"\n",
    "    takes path is Text\n",
    "    gives Text\n",
    "state n is client Whole starting 1\n",
    "state out is client Text from draw with level is n\n",
    "state contents is server Text from readAt with path is \"in.txt\"\n",
    "view\n",
    "    Column\n",
    "        Text out\n",
    "        when contents\n",
    "            Loading           show Text \"…\"\n",
    "            Failed with error show Text error.message\n",
    "            Ready with body   show Text body\n",
);

/// The destinations a deployment of `source` reports, for `target`.
fn destinations(source: &str, target: Target) -> Vec<String> {
    let bundle = compile_source(source);
    let program = program(&bundle);
    let deployment = generate(&program, &Options::new(target, "linked-app"))
        .unwrap_or_else(|refusal| panic!("{target:?} refused: {}", refusal.message));
    deployment
        .linked_modules
        .iter()
        .map(|module| module.destination.clone())
        .collect()
}

/// **Every target ships both halves' modules, each beside its importer.**
///
/// The browser half is displaced by the deploy layout and the endpoints are
/// not: `client.js` moves to `public/` while `functions/greeting.js` stays
/// where the bundle put it. The emitted `import` is the author's specifier
/// verbatim, so `./draw.js` beside `public/client.js` and `./io.js` beside
/// `functions/contents.js` are two different files in two directories.
#[test]
fn every_target_places_each_module_beside_the_file_that_imports_it() {
    // Written out rather than computed from `browser_root`, which is the
    // function under test: an expectation that asks the implementation
    // where it put the file agrees with it by construction. The four
    // happen to say the same thing today, and each is here separately so
    // that one of them changing is one line of this table.
    let expected = [
        (Target::Cloudflare, "public/draw.js"),
        (Target::Lambda, "public/draw.js"),
        (Target::Vercel, "public/draw.js"),
        (Target::Deno, "public/draw.js"),
    ];

    // Sized as well as looped: a per-target assertion made only inside the
    // loop is satisfied by an empty table, which is what a refactor that
    // dropped a target would leave behind.
    assert_eq!(expected.len(), Target::ALL.len());
    for (target, browser) in expected {
        assert_eq!(
            destinations(BOTH_HALVES, target),
            [browser.to_string(), "functions/io.js".to_string()],
            "{target:?} put a module somewhere its import does not name"
        );
    }
}

/// A specifier both halves import is two files, not one shared module.
///
/// §16.3.12's invariant 4 keeps an import edge from existing between the
/// tiers, and the duplication is the price of that — the same trade
/// §16.3.12 already accepts for a colourless function. A deployment that
/// shipped one copy would have to pick a directory, and the other half's
/// import would not resolve to it.
#[test]
fn a_module_both_halves_import_is_shipped_to_both() {
    let destinations = destinations(
        concat!(
            "foreign clean is anywhere\n",
            "    from \"./util.js\" as \"clean\"\n",
            "    takes text is Text\n",
            "    gives Text\n",
            "state typed is client Text starting \"hi\"\n",
            "state shown is client Text from clean with text is typed\n",
            "state stored is server Text from clean with text is \"x\"\n",
            "view\n",
            "    Column\n",
            "        Text shown\n",
            "        when stored\n",
            "            Loading           show Text \"…\"\n",
            "            Failed with error show Text error.message\n",
            "            Ready with body   show Text body\n",
        ),
        Target::Cloudflare,
    );

    // Both entries carry the same specifier, so the order is the
    // destinations' own — which is the bundle's ordering, not a claim
    // about which half is shipped first.
    assert_eq!(destinations, ["functions/util.js", "public/util.js"]);
}

/// A bare specifier names a package the platform resolves, not a file this
/// deployment owns. Copying one would mean guessing where it lives.
#[test]
fn a_package_specifier_is_not_shipped() {
    let destinations = destinations(
        concat!(
            "foreign parse is anywhere\n",
            "    from \"marked\" as \"parse\"\n",
            "    takes source is Text\n",
            "    gives Text\n",
            "state body is client Text starting \"hi\"\n",
            "state out is client Text from parse with source is body\n",
            "view\n",
            "    Column\n",
            "        Text out\n",
        ),
        Target::Vercel,
    );

    assert!(destinations.is_empty(), "{destinations:?}");
}

/// A program with no `foreign` reports nothing to ship, on every target.
/// The empty case is worth pinning: a deployment that invented a copy for
/// a program that imports nothing would be shipping the project's files on
/// no evidence at all.
#[test]
fn a_program_with_no_foreign_ships_no_modules() {
    let bundle = support::compile_example("examples/guestbook.zd");
    assert_eq!(Target::ALL.len(), 4);
    for target in Target::ALL {
        let program = program(&bundle);
        let deployment = generate(&program, &Options::new(target, "guestbook"))
            .unwrap_or_else(|refusal| panic!("{target:?} refused: {}", refusal.message));
        assert!(
            deployment.linked_modules.is_empty(),
            "{target:?}: {:?}",
            deployment.linked_modules
        );
    }
}

/// A destination is a path inside the deployment, and the generated files
/// are already held to that rule. A copied one is held to it here for the
/// same reason: the destination decides where the CLI writes, and a `..`
/// in it would write outside the directory the user named.
#[test]
fn every_destination_stays_inside_the_deployment() {
    assert_eq!(Target::ALL.len(), 4);
    for target in Target::ALL {
        let bundle = compile_source(BOTH_HALVES);
        let program = program(&bundle);
        let deployment = generate(&program, &Options::new(target, "linked-app")).unwrap();
        let checked: BTreeSet<&str> = deployment
            .linked_modules
            .iter()
            .map(|module| module.destination.as_str())
            .collect();
        assert_eq!(checked.len(), 2, "{target:?}: {checked:?}");
        for destination in checked {
            assert!(!destination.starts_with('/'), "{target:?}: {destination}");
            assert!(
                !destination.split('/').any(|segment| segment == ".."),
                "{target:?}: {destination}"
            );
        }
    }
}
