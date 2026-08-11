//! Every example that parses must also resolve, and there is no longer an
//! exception.
//!
//! `blog.zd` was excluded because it used syntax that was designed but not
//! implemented. `static` landed, and its `readMarkdown "content/blog"` —
//! a call with a bare argument, which has no production in §4.4 — was
//! respelled by the spec into the `build` capability form. `EXPECTED` is
//! therefore every `.zd` file in `examples/`, so a new example is a test
//! failure until it is named here.
//!
//! Resolution is against the prelude, exactly as `zdc check` does it
//! (§17.4.1): an example that calls `atOr` resolves only if the library
//! is beneath it, and testing without one would be testing a pipeline
//! nothing runs.
//!
//! Keeping the rest under test stops the examples rotting as the compiler
//! grows: resolution is the first pass that checks names, and adding it
//! found two examples whose pipelines read a signal nobody declared.
const EXCLUDED: &[&str] = &[];

/// The examples that must resolve, named so that deleting or renaming one
/// is a test failure rather than a silently smaller run.
const EXPECTED: &[&str] = &[
    "blog.zd",
    // The two typed fields (#45, #48). What it demonstrates is the
    // *types*: both bind an `Option`, because a box with nothing usable
    // in it has no number in it, and a date is the moment
    // `prelude/time.zd` already reads apart rather than a type the
    // language does not have.
    "booking.zd",
    "components.zd",
    "content.zd",
    "counter.zd",
    "disclosure.zd",
    "dungeon.zd",
    // Levenshtein over a flat table, because there is no two-dimensional
    // structure to fill (#195).
    "edit-distance.zd",
    "events.zd",
    // The DOM-owning FFI (§14E.1): a foreign written as a view element.
    "gauge.zd",
    // The six algorithm examples. Their answers are pinned by running
    // them, in `zdc-codegen/tests/algorithms.rs`; what is asserted here
    // is only that they resolve against the prelude.
    "graph-traversal.zd",
    "guestbook.zd",
    "hello.zd",
    "knapsack.zd",
    // The two components `blog.zd` composes its pages out of. A module is
    // a unit of naming rather than of deployment (§14D.2).
    "layout.zd",
    "leaderboard.zd",
    "model.zd",
    "page.zd",
    "poker.zd",
    // The two things the browser owns and the program could not ask
    // about: `remembered` state, which survives a reload and belongs to
    // one browser, and `media`, which is what the visitor's display asks
    // for.
    "preferences.zd",
    "queens.zd",
    "shortest-path.zd",
    "site.zd",
    // Six claims about the file below it, run by `zdc test` (#169). It is
    // listed here for the same reason every other file is: a `test` is an
    // ordinary declaration, so a file holding one resolves against the
    // prelude by the ordinary path and must go on doing so.
    "sorting.test.zd",
    "sorting.zd",
    // The only example that stores something other than a number. It was
    // added because there was none: `JSON.stringify(new Map(...))` is
    // `{}`, so every `durable Map` silently stored an empty object, and
    // with no example exercising that path nothing ever noticed.
    "tally.zd",
    "terminal-help.zd",
    "todo.zd",
    "voting-board.zd",
    "writing.zd",
];

fn examples() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Resolution goes through the module loader rather than through `parse`
/// alone.
///
/// A file with a `use` line is not a whole program on its own (§14D.2), and
/// the loader is what turns the entry file and everything it reaches into
/// one. Files with no imports take the same route and link to themselves,
/// so there is one path rather than two that could disagree.
#[test]
fn every_parseable_example_also_resolves() {
    let dir = examples();
    let mut resolved = Vec::new();

    for entry in std::fs::read_dir(&dir).expect("examples directory") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("zd") {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("utf-8 file name")
            .to_string();
        if EXCLUDED.contains(&name.as_str()) {
            continue;
        }

        let linked = zdc_resolve::load(&path).unwrap_or_else(|failure| {
            panic!("{name} failed to load: {}", failure.errors[0].message)
        });

        let prelude = zdc_lib::load();
        match zdc_resolve::Resolver::linked_with_prelude(prelude.program(), &linked).resolve() {
            Ok(_) => resolved.push(name),
            Err(errors) => panic!(
                "{name} failed to resolve, {} error(s), the first being: {}",
                errors.len(),
                errors[0].message
            ),
        }
    }

    resolved.sort();
    assert_eq!(resolved, EXPECTED);
}

/// Nothing is excluded, and that is asserted rather than assumed.
///
/// This test used to check that the excluded example was still beyond the
/// grammar, so that an exclusion could not outlive its cause. It is kept,
/// inverted: the list is empty, and any name added back to it must first
/// fail to parse.
#[test]
fn no_example_is_excluded_that_the_grammar_can_reach() {
    for name in EXCLUDED {
        let src = std::fs::read_to_string(examples().join(name)).expect("read");
        assert!(
            zdc_parser::parse(&src).is_err(),
            "{name} now parses, so it no longer needs excluding"
        );
    }
}

/// `examples/tree-webgl/` is the acceptance criterion for #271 stage 3: a
/// real library driven from the language with no hand-written JavaScript.
///
/// The same three assertions the CSS tree gets, and for the same reason —
/// either half alone would pass while the point of the example was lost.
/// It typechecks, the directory holds no `.js`, and the file actually
/// uses all three of the forms that made it possible: a property read off
/// a handle, a `foreign` that gives nothing, and a handle held in
/// `client` state acquired with `starting`.
#[test]
fn the_webgl_tree_typechecks_and_ships_no_javascript() {
    let dir = examples().join("tree-webgl");
    let path = dir.join("webgl.zd");

    let linked = zdc_resolve::load(&path).expect("webgl.zd links");
    let prelude = zdc_lib::load();
    let hir = zdc_resolve::Resolver::linked_with_prelude(prelude.program(), &linked)
        .resolve()
        .unwrap_or_else(|errors| panic!("webgl.zd failed to resolve: {}", errors[0].message));
    let split = zdc_graph::split(&hir);
    if let Some(error) = split.errors().next() {
        panic!("webgl.zd was rejected by the split: {}", error.message);
    }
    if let Err(errors) = zdc_types::check(&hir, &split) {
        panic!("webgl.zd failed to typecheck: {}", errors[0].message);
    }

    let mut javascript = Vec::new();
    let mut pending = vec![dir.clone()];
    while let Some(here) = pending.pop() {
        for entry in std::fs::read_dir(&here).expect("the WebGL example's directory") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("js") {
                javascript.push(path);
            }
        }
    }
    assert!(
        javascript.is_empty(),
        "the point of this example is that three.js is driven with no JavaScript beside it, \
         and it now ships {javascript:?}"
    );

    let source = std::fs::read_to_string(&path).expect("read webgl.zd");
    for form in [
        "of Handle as",
        "gives nothing",
        "\n    do ",
        "Handle starting",
    ] {
        assert!(
            source.contains(form),
            "the example no longer demonstrates `{form}`, which is what it is here for"
        );
    }
}

/// `components.zd` is the acceptance criterion for §14D, so it is named
/// here rather than only counted among the rest: it must typecheck, not
/// merely resolve.
///
/// The split runs first, in §17.1.2's order, because the type of a
/// cross-placement read depends on the crossing. Asserting it produced no
/// error is part of the criterion: a component whose state or arguments the
/// placement pass rejected has not been accepted.
#[test]
fn the_components_example_typechecks() {
    let path = examples().join("components.zd");
    let linked = zdc_resolve::load(&path).expect("components.zd links");
    let hir = zdc_resolve::Resolver::linked(&linked)
        .resolve()
        .unwrap_or_else(|errors| panic!("components.zd failed to resolve: {}", errors[0].message));
    let split = zdc_graph::split(&hir);
    if let Some(error) = split.errors().next() {
        panic!("components.zd was rejected by the split: {}", error.message);
    }
    if let Err(errors) = zdc_types::check(&hir, &split) {
        panic!("components.zd failed to typecheck: {}", errors[0].message);
    }
}

/// `examples/tree/` is the one example in a directory of its own, and the
/// walk above does not reach it: `read_dir` on `examples/` yields the
/// directory, whose extension is not `zd`, and stops there. It was
/// therefore the one example under no test at all.
///
/// It is named here for the property it exists to demonstrate. The tree
/// used to hand four parallel lists to 204 lines of three.js behind a
/// `foreign … gives view`, on the argument that a branch's position is
/// `sin` and `cos` of an angle and this language has neither. It does not
/// any more, because a branch is now a child element of the branch it
/// grows from and the composition of the rotations is the browser's: what
/// the program computes is which branches exist, and what
/// `assets/tree.css` turns into a shape is ten class names.
///
/// So the assertion is both halves — that it still typechecks, and that
/// the directory has no JavaScript in it. Either alone would pass while
/// the point of the example was lost.
#[test]
fn the_tree_example_typechecks_and_ships_no_javascript() {
    let dir = examples().join("tree");
    let path = dir.join("tree.zd");

    let linked = zdc_resolve::load(&path).expect("tree.zd links");
    let prelude = zdc_lib::load();
    let hir = zdc_resolve::Resolver::linked_with_prelude(prelude.program(), &linked)
        .resolve()
        .unwrap_or_else(|errors| panic!("tree.zd failed to resolve: {}", errors[0].message));
    let split = zdc_graph::split(&hir);
    if let Some(error) = split.errors().next() {
        panic!("tree.zd was rejected by the split: {}", error.message);
    }
    if let Err(errors) = zdc_types::check(&hir, &split) {
        panic!("tree.zd failed to typecheck: {}", errors[0].message);
    }

    let mut javascript = Vec::new();
    let mut pending = vec![dir.clone()];
    while let Some(here) = pending.pop() {
        for entry in std::fs::read_dir(&here).expect("the tree example's directory") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("js") | Some("mjs") | Some("cjs") | Some("ts")
            ) {
                javascript.push(path.display().to_string());
            }
        }
    }
    assert!(
        javascript.is_empty(),
        "the tree example is the demonstration that this needs no JavaScript, \
         and it now ships some: {javascript:?}"
    );
}
