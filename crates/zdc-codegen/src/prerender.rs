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
//! client builds the same tree whether or not it finds one already
//! there, which is what makes the pass safe to skip.
//!
//! # What this is not, and the difference matters
//!
//! **This is a first paint, not hydration.** The client does not take the
//! painted tree over — it mounts its own on top, and `view.rs` argues why
//! at the root emission: a region's two anchor comments are adjacent in a
//! clone and are not in a served document, so the emitted walk cannot find
//! where a region ends. Adopting a served tree needs anchors a walk can
//! match, which is issue #208's third emission mode.
//!
//! The distinction is worth keeping straight because the *reader's*
//! experience is the same either way — the document arrives painted, and
//! the replacement happens in the task that loaded the module, before any
//! paint of its own. What adoption would save is the rebuilding, which is
//! work rather than anything visible.

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
    for (_, source) in linked {
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
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}
