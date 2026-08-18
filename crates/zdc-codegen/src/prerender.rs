//! The first paint, computed on the build host.
//!
//! # The flash this removes
//!
//! §16.3.1's document is a `<div id=app>` and a module that fills it. So
//! nothing is on the page until the script has been fetched, parsed and
//! run, and the reader sees the shell — a blank one — for however long
//! that takes. On a fast connection it is a flicker; on a slow one it is
//! the whole page arriving at once, several seconds in, with nothing
//! before it.
//!
//! Nothing about that is necessary. The program is already evaluated on
//! the build host for every `static` value it holds, and the same
//! evaluator can be handed a DOM: `dom-shim.js` models the tree,
//! `prerender.js` stubs the rest of the browser, and the emitted module's
//! own `main` builds the page. What comes back is markup, and markup goes
//! in the shell.
//!
//! # Why it is best-effort and never fatal
//!
//! A program that cannot be prerendered still has to build. Every reason
//! one might not — a `foreign` reaching for a package the build host has
//! no copy of, a `view` that reads something no stub models, an engine
//! budget exhausted by a deep fold — is a reason to ship the document
//! that was shipped before this existed, and none of them is a reason to
//! refuse the program. So a failure here returns `None` and the build
//! carries on.
//!
//! That also means the prerendered markup is never *depended* on. The
//! client renders the same tree whether or not it finds one already
//! there, which is what makes the pass safe to skip.
//!
//! # The client takes this tree over rather than replacing it (#208)
//!
//! It did not always. The first version of this pass painted a document
//! and the client then mounted its own tree over the top — the reader saw
//! the same thing either way, and what was thrown away was the work rather
//! than the picture.
//!
//! Adoption is the third emission mode, and it is `adopt.js` plus two
//! changes that meet there: a region's anchors are distinguishable in the
//! markup (`Edge` in `view.rs`), and every served region is lifted out
//! from between them before any walk runs. `view.rs`'s `root_template`
//! carries the argument in full.
//!
//! What this pass owes that mode is **exactness**: the markup below is
//! parsed by a browser, and the walk is indexed against the templates it
//! came from. `dom-shim.js` models no insertion modes, so no test here can
//! settle whether the two agree —
//! `zdc-cli/tests/browser.rs::a_prerendered_page_is_adopted_by_the_client_rather_than_rebuilt`
//! is the only authority, and it asks a real Chrome. When they disagree
//! the page still renders: a region whose served nodes do not fit is built
//! instead of adopted, which costs the work and never the correctness.

/// A document's markup, ready to go inside the shell's container.
pub struct Prerendered {
    pub html: String,
}

/// Run `client_js` against a shimmed DOM and return what it painted.
///
/// `linked` is the runtime modules the bundle imports, in dependency
/// order — the same list the bundle writes to disk, because a module the
/// program links is a module the prerender needs.
pub fn prerender(client_js: &str, linked: &[(&str, &str)]) -> Option<Prerendered> {
    let mut sandbox = zdc_runtime::Sandbox::new();

    // Order matters and is the same order a browser establishes: the
    // globals a module might touch while it evaluates, then the document,
    // then the runtime, then the program.
    for source in [zdc_runtime::PRERENDER_JS, zdc_runtime::DOM_SHIM_JS] {
        sandbox.load(&flattened(source)).ok()?;
    }
    for (_, source) in in_dependency_order(linked) {
        sandbox.load(&flattened(source)).ok()?;
    }
    sandbox.load(&flattened(client_js)).ok()?;

    // A **fragment** and not an element, because the shim's `innerHTML`
    // serialises the node it is asked about and a fragment serialises to
    // its children alone. An element would have wrapped the whole page in
    // one more `<div>` than the client builds — which is not a cosmetic
    // difference: the emitted walk indexes from the container's first
    // child, so hydration would attach every binding one level out.
    //
    // It is detached either way, so nothing `main` does can reach the
    // shim's document.
    let html = sandbox
        .text("(() => { const $c = document.createDocumentFragment(); main($c); return $c.innerHTML; })()")
        .ok()?;
    if html.is_empty() {
        return None;
    }
    Some(Prerendered { html })
}

/// A module as a script: no `import`, no `export`.
///
/// The engine has no module loader — the one it ships wants a filesystem
/// resolver — and a script's top-level `const`s stay where the next
/// `eval` can see them, which is the whole interface this pass needs.
/// The same two lines `zdc-runtime`'s own suites use.
fn flattened(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            // An import is dropped, because every module is already in this
            // one scope — except for the names it renames. `import { request
            // as $request }` binds `$request`, and dropping the line leaves
            // the emitted code calling a name nothing declares. That is a
            // `ReferenceError` at load, which `prerender` turns into `None`,
            // which is a first paint silently not taken: every program with a
            // `request` or a server read emits exactly this shape.
            if line.trim_start().starts_with("import ") {
                return aliases_of(line);
            }
            line.strip_prefix("export ").unwrap_or(line).to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `linked`, reordered so a module's imports load before it.
///
/// The parameter above says "in dependency order" and the caller passes
/// alphabetical — which is the order the bundle writes to disk, and there
/// it does not matter, because a browser is given the graph and resolves
/// it. Here every module is one script in one scope, so a name has to
/// exist by the time the line binding it runs. `store.js` binds
/// `decode as decodeValue`, `wire.js` sorts after `store.js`, and the
/// binding was reading a name nothing had declared yet.
///
/// Depth-first, and a module already placed is skipped, so a cycle stops
/// rather than spins. A cycle cannot be ordered at all and this pass is
/// allowed to fail — leaving it to fail at load is better than inventing
/// an order for it here.
fn in_dependency_order<'a>(linked: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut placed: Vec<(&str, &str)> = Vec::with_capacity(linked.len());
    let mut seen: Vec<&str> = Vec::with_capacity(linked.len());
    for (name, _) in linked {
        place(name, linked, &mut placed, &mut seen);
    }
    placed
}

/// One module and everything it imports, deepest first.
fn place<'a>(
    name: &'a str,
    linked: &[(&'a str, &'a str)],
    placed: &mut Vec<(&'a str, &'a str)>,
    seen: &mut Vec<&'a str>,
) {
    if seen.contains(&name) {
        return;
    }
    seen.push(name);
    let Some((_, source)) = linked.iter().find(|(each, _)| *each == name) else {
        return;
    };
    for line in source.lines() {
        let Some(rest) = line.trim_start().strip_prefix("import ") else {
            continue;
        };
        // `from './wire.js'` — the only import shape a runtime module
        // writes, and `MODULES` names them all with a `runtime/` prefix.
        let Some(open) = rest.find("'./") else {
            continue;
        };
        let after = &rest[open + 3..];
        let Some(close) = after.find('\'') else {
            continue;
        };
        let needed = format!("runtime/{}", &after[..close]);
        if let Some((each, _)) = linked.iter().find(|(each, _)| *each == needed) {
            place(each, linked, placed, seen);
        }
    }
    placed.push((name, source));
}

/// The `const` lines an import's renames need, and nothing for the rest.
///
/// Only `{ a as b }` needs one. A plain `{ a }` binds the name the module
/// already declares, and a scope holding both would be a redeclaration
/// rather than a repair.
fn aliases_of(line: &str) -> String {
    let Some(open) = line.find('{') else {
        return String::new();
    };
    let Some(close) = line[open..].find('}') else {
        return String::new();
    };
    line[open + 1..open + close]
        .split(',')
        .filter_map(|clause| clause.split_once(" as "))
        .map(|(from, to)| format!("const {} = {};", to.trim(), from.trim()))
        .collect::<Vec<_>>()
        .join(" ")
}
