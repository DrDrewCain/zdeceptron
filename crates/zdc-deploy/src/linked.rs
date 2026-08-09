//! Where each `foreign` module lands in a deployment, as opposed to in a
//! bundle (#225).
//!
//! A bundle and a deployment are two different trees. `zdc build` writes
//! `client.js` at the root; `zdc deploy` writes it under `public/`, because
//! that is where every target's static handling looks — Cloudflare's
//! `[assets]` directory, Vercel's `outputDirectory`, the Deno entry's own
//! file read, and the directory Lambda's report tells you to put behind
//! CloudFront. The endpoints move nowhere: they are written at
//! `functions/<name>.js` in both trees, which is what `_zd/endpoints.js`
//! imports as `../functions/<name>.js`.
//!
//! So exactly one of the two halves is displaced, and only that half's
//! modules need re-placing. The emitted `import` is the author's specifier
//! verbatim — nothing rewrites it — so a module has to sit beside whichever
//! file imports it, wherever *that* file ended up.
//!
//! Nothing here reads or writes a file. This crate takes a compiled bundle
//! as data and hands back paths and contents, and the same division holds
//! for a copied module as for a generated one: the adapter says where it
//! goes, the CLI copies it under the sandbox rule that governs every path a
//! program names.

use std::collections::BTreeSet;

use zdc_codegen::LinkedModule;

use crate::{Program, Target};

/// Every `foreign` module this deployment has to contain, with each
/// destination relative to the **deployment** root.
///
/// The two halves are told apart by the data rather than by inspecting the
/// destination string: a module an endpoint imports appears in that
/// endpoint's own [`ServerFunction::linked`](zdc_codegen::ServerFunction),
/// and everything else the bundle reported belongs to the browser half. A
/// specifier imported by both halves is two entries with two destinations
/// and is shipped twice, which is what the bundle already says and what
/// keeps each import resolving without a shared module between the tiers.
pub fn place(program: &Program<'_>, target: Target) -> BTreeSet<LinkedModule> {
    let endpoints: BTreeSet<LinkedModule> = program
        .functions
        .iter()
        .flat_map(|function| function.linked.iter().cloned())
        .collect();

    let root = target.browser_root();
    let browser = program
        .linked
        .iter()
        .filter(|module| !endpoints.contains(module))
        .map(|module| LinkedModule {
            specifier: module.specifier.clone(),
            // `destination` is relative to the bundle root, and the bundle
            // root is `public/` here. Prefixing it is the whole of the
            // translation, and it is done on the destination rather than
            // recomputed from the specifier so that the two cannot drift.
            destination: format!("{root}/{}", module.destination),
        });

    endpoints.iter().cloned().chain(browser).collect()
}
