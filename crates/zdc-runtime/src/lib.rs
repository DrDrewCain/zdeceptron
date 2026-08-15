//! The ZDeceptron JavaScript runtime, embedded and executable from Rust.
//!
//! The runtime library that generated code links against is written in
//! JavaScript, because it manipulates the DOM and there is no other way to
//! do that (spec §14E.2). But *verifying* it must not require a JavaScript
//! toolchain: needing Node to build ZDeceptron would be the first crack in
//! the claim that a developer installs one binary and nothing else.
//!
//! So the sources are embedded here and evaluated with a pure-Rust engine.
//! `cargo test` covers the runtime; nothing else has to be installed.
//!
//! # Two halves, and why the seam is a feature
//!
//! Holding the runtime sources and *running* them are different jobs, and
//! the `evaluate` feature is where they part. Everything above the seam is
//! text and plain data — the `.js` and `.css` a bundle ships, and the
//! signatures a capability is written against. Everything below it is
//! `boa_engine`, a JavaScript interpreter written in Rust.
//!
//! The seam is cut here rather than left implicit because `boa_engine`
//! reaches `getrandom`, and `getrandom` will not build for
//! `wasm32-unknown-unknown` without an entropy backend chosen by `--cfg`.
//! `zdc-codegen` depends on this crate for seven `const &str`s, so without
//! the seam the entire front end inherited that refusal and no part of this
//! compiler could run in a browser (#171). See the feature's comment in
//! `Cargo.toml` for the dependency chain in full.
#![forbid(unsafe_code)]

pub mod minify;

use std::borrow::Cow;
use std::path::Path;

#[cfg(feature = "evaluate")]
use std::path::PathBuf;

#[cfg(feature = "evaluate")]
use boa_engine::object::builtins::JsArray;
#[cfg(feature = "evaluate")]
use boa_engine::{
    js_string, Context, JsError, JsNativeError, JsNativeErrorKind, JsResult, JsValue,
    NativeFunction, Source,
};

/// The reactivity core: signals, derived values, effects, batching.
pub const SIGNAL_JS: &str = include_str!("../runtime/signal.js");

/// The minimal DOM the runtime and everything downstream of it run
/// against when there is no browser.
///
/// Exposed from here because four crates were reaching for it and only one
/// of them owns it. They used to reach across the workspace with
/// `../../zdc-runtime/tests/dom-shim.js`, which works in a workspace build
/// and does not survive `cargo package`: a crate may only embed files
/// inside its own directory. One copy, one owner, and a published
/// `zdc-runtime` that compiles.
pub const DOM_SHIM_JS: &str = include_str!("../runtime/dom-shim.js");

/// DOM rendering. Requires a document, so it is embedded for shipping
/// rather than for evaluation here.
pub const DOM_JS: &str = include_str!("../runtime/dom.js");

/// The lifecycle of a `foreign … gives view`: create, update, destroy.
///
/// Its own module rather than part of `dom.js` because a DOM-owning
/// foreign is optional and its machinery is not small: a program that
/// writes none must not download it (§16.3.1). It imports `signal.js` and
/// nothing else — the node is handed in, so there is no DOM dependency.
pub const FOREIGN_JS: &str = include_str!("../runtime/foreign.js");

/// The `Prose` render path — the one function in the runtime that parses
/// HTML. Its own module so a program with no `Prose` does not ship it.
pub const MARKUP_JS: &str = include_str!("../runtime/markup.js");

/// Keyed list reconciliation: `each`, `eachInto` and the interim key
/// function.
///
/// Its own module for the reason `foreign.js` and `markup.js` are: a
/// program with no list must not download a reconciler it never calls
/// (§16.3.1), and the minimal-move reconciler §16.10 scheduled is the
/// largest single thing the renderer contains. It imports `signal.js` and
/// one function from `dom.js`, both of which a program with a list has
/// already linked.
pub const LIST_JS: &str = include_str!("../runtime/list.js");

/// The `remembered` placement's store: `localStorage`, as a signal.
///
/// Its own module for the reason `foreign.js`, `markup.js` and `list.js`
/// are: a program that declares no `remembered` state must not download a
/// store wrapper it never calls (§16.3.1). It imports `signal.js` and
/// `wire.js` — the same encoding a `durable` value uses for the same trip,
/// because `JSON.stringify` turns a `Map` into `{}` here exactly as it
/// does there.
pub const REMEMBERED_JS: &str = include_str!("../runtime/remembered.js");

/// `media "…"` — a CSS media query, as a signal that changes with it.
///
/// Its own module, and it imports `signal.js` and nothing else: a program
/// that asks the browser no question must not ship a `matchMedia`
/// subscription (§16.3.1).
pub const MEDIA_JS: &str = include_str!("../runtime/media.js");

/// The clock: `every "250ms"`, `every frame` and `after "2s"`.
///
/// Its own module for the same reason as the modules above, and the size
/// gate is the reason it is not in `signal.js`: a null program links
/// `signal.js`, so anything put there is shipped to every program forever.
/// It imports `signal.js` and nothing else — a clock writes a cell and
/// touches no DOM.
pub const CLOCK_JS: &str = include_str!("../runtime/clock.js");

/// Document key listeners: `on key "Escape"`.
///
/// Its own module for the reason the modules above are: a program that
/// writes no `on key` must not download it (§16.3.1).
/// It imports `signal.js` and nothing else — it needs a listener and a
/// focus question rather than a node to render into.
pub const KEYS_JS: &str = include_str!("../runtime/keys.js");

/// The outbound request a `request` declaration is (#19).
///
/// Its own module for the reason the modules above are theirs, and with
/// more riding on it: a program that declares no
/// `request` must not ship the one `fetch` in the runtime that can name a
/// host it was not given. It imports `signal.js` and nothing else.
pub const REQUEST_JS: &str = include_str!("../runtime/request.js");

/// The client half of the derived boundary: `$remote` and `$call`.
///
/// A bundle links against this only when the split found a crossing, so a
/// client-only program still ships nothing it does not use (§16.3.1).
pub const RPC_JS: &str = include_str!("../runtime/rpc.js");

/// The wire format: how a ZD value survives JSON.
///
/// Its own module because three separate things encode and decode with it
/// — the browser, the platform adapter, and the live-sync stream — and a
/// second copy of the rules is how they come to disagree.
pub const WIRE_JS: &str = include_str!("../runtime/wire.js");

/// Live sync for `durable` placement, and the transport seam it needs.
///
/// Shipped only when the split found a durable key. It imports `rpc.js`,
/// which a program with a crossing already has.
pub const STORE_JS: &str = include_str!("../runtime/store.js");

/// The built-in view elements.
pub const ELEMENTS_JS: &str = include_str!("../runtime/elements.js");

/// The base styling of the built-in elements, as classes.
///
/// Spec §16.2 R6: `Column` and `Row` carry `zd-col`/`zd-row` rather than an
/// inline style object, so the declarations have to ship somewhere. This is
/// the base layer of the `styles.css` a build emits.
pub const BASE_CSS: &str = include_str!("../runtime/base.css");

/// Which build a runtime module is being emitted for — spec §16.3.1's
/// "ships nothing it does not use", applied to the checks themselves.
///
/// # Why there are two builds at all
///
/// Several of the defects this repository has found were invisible to
/// every static pass and visible only in an emitted program's answer: a
/// durable `Map` serialised to `{}` (#204), a `switch` fell through. A
/// runtime that checks its own invariants is where that class is caught
/// next time. But a check that runs in production is a check a reader
/// downloads and pays for on every event, and the size gate in
/// `crates/zdc-bench/tests/scaling.rs` is measured in single-digit bytes
/// of headroom — so an assertion that could not be removed would have to
/// be argued against on size, one at a time, forever.
///
/// So the assertions are marked and the release build removes them. What
/// makes that safe rather than a second source of truth is the marker's
/// shape: it delimits *whole lines*, so what a release build ships is a
/// subsequence of the lines a developer reads and tests, and
/// `the_release_runtime_still_passes_the_suite` in `tests/render.rs` runs
/// the stripped source through the same suite as the unstripped one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Keep the assertions. `zdc dev` builds this.
    Development,
    /// Remove them. `zdc build` builds this, and it is the default,
    /// because the failure that costs a reader bytes must be the one that
    /// takes an explicit decision to cause.
    #[default]
    Release,
}

/// The line that opens a block only a development build carries.
pub const DEV_OPEN: &str = "// $dev";

/// The line that closes one.
pub const DEV_CLOSE: &str = "// $end";

/// One runtime module's source, as the given build ships it.
///
/// Two transformations for a release build, and **the order between them
/// is a correctness requirement, not a style** (issue #135). A `// $dev`
/// marker is a comment, and minification removes comments — so minifying
/// first would delete both markers and leave the assertions they delimit
/// in the file, shipping to every reader exactly the code the mechanism
/// exists to remove. Stripping runs first, always.
///
/// A development build is neither stripped nor minified: `zdc dev` serves
/// it, and a reader of *that* build is the developer who wrote the
/// program, standing in a debugger.
pub fn for_mode(source: &'static str, mode: Mode) -> Cow<'static, str> {
    match mode {
        Mode::Development => Cow::Borrowed(source),
        Mode::Release => Cow::Owned(minify::javascript(&strip_dev_blocks(source))),
    }
}

/// Drop every `// $dev` … `// $end` block, markers included.
///
/// Whole lines and no nesting: a nested block would need a depth counter
/// here and would let a reader mis-count which `// $end` closes what, and
/// no assertion has wanted one. `dev_blocks_are_balanced` fails the build
/// if a module ever writes one, rather than this function guessing.
fn strip_dev_blocks(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut inside = false;
    for line in source.lines() {
        match line.trim() {
            DEV_OPEN => inside = true,
            DEV_CLOSE => inside = false,
            _ if !inside => {
                out.push_str(line);
                out.push('\n');
            }
            _ => {}
        }
    }
    out
}

/// Every embedded runtime module, by the path a bundle writes it to.
///
/// One list, so a module added to this crate is covered by the marker
/// check and by the size survey without anyone remembering to add it
/// twice.
pub const MODULES: &[(&str, &str)] = &[
    ("runtime/signal.js", SIGNAL_JS),
    ("runtime/dom.js", DOM_JS),
    ("runtime/foreign.js", FOREIGN_JS),
    ("runtime/markup.js", MARKUP_JS),
    ("runtime/keys.js", KEYS_JS),
    ("runtime/wire.js", WIRE_JS),
    // `list.js` was missing from this list, which is the exact failure the
    // doc comment above promises it prevents: it carries two `// $dev`
    // blocks, and an unbalanced marker in an unlisted module deletes the
    // rest of that file from every release build with nothing to say so.
    ("runtime/list.js", LIST_JS),
    ("runtime/request.js", REQUEST_JS),
    ("runtime/rpc.js", RPC_JS),
    ("runtime/store.js", STORE_JS),
    ("runtime/elements.js", ELEMENTS_JS),
];

/// An evaluation failure, with the engine's own message.
#[derive(Debug)]
pub struct RuntimeError {
    pub message: String,
    /// `true` when the engine stopped the program rather than the program
    /// stopping itself: a loop that never ends, or recursion that never
    /// bottoms out. The two need different diagnostics, because one is a
    /// mistake in the program and the other is a mistake about what a
    /// build is allowed to do.
    pub budget_exceeded: bool,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RuntimeError {}

#[cfg(feature = "evaluate")]
impl From<JsError> for RuntimeError {
    fn from(error: JsError) -> Self {
        let budget_exceeded = matches!(
            error.as_native().map(|native| &native.kind),
            Some(JsNativeErrorKind::RuntimeLimit)
        );
        RuntimeError {
            message: error.to_string(),
            budget_exceeded,
        }
    }
}

/// How much work one evaluation may do before the engine stops it.
///
/// A bound, not a timeout. §17.4.8 reached for a wall-clock budget because
/// it assumed the code would run in someone else's process, where there is
/// nothing to meter; in an engine the compiler owns there is, and a bound
/// is strictly better. It is **deterministic** — the same program fails on
/// a slow machine and a fast one alike — and §14A.4 cannot tolerate a
/// build failure that depends on how busy the host was, which is the same
/// argument §17.4.7 makes against seeding a parity test randomly.
///
/// Every non-terminating JavaScript program loops or recurses, so bounding
/// both bounds termination.
#[cfg(feature = "evaluate")]
const LOOP_ITERATION_BUDGET: u64 = 10_000_000;

/// What a capability may answer with.
///
/// Three shapes, because the closed set has three result types and a
/// fourth would be a design decision rather than a convenience. There is
/// no `Object` here on purpose: a capability that could return arbitrary
/// structure would be a module loader with extra steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provided {
    Text(String),
    /// HTML, from the one capability that produces it.
    ///
    /// It carries a `String` exactly as `Text` does, and crosses into the
    /// engine as the same JavaScript string — the distinction is not a
    /// runtime representation, it is the compiler's `Type::Markup`, which
    /// is what decides whether a value may reach the one element that
    /// parses HTML. Keeping the variant separate here means the answer to
    /// "which capability produced HTML" is in the type of the answer
    /// rather than in a comment.
    Markup(String),
    List(Vec<String>),
}

/// One capability the compiler answers for the code it is running.
///
/// `answer` is a plain function pointer, not a closure: the only state a
/// capability may consult is the project root it is handed, so there is
/// nowhere for ambient authority to hide.
#[derive(Clone, Copy)]
pub struct Capability {
    pub name: &'static str,
    pub answer: fn(&Path, &str) -> Result<Provided, String>,
}

/// The global a capability is registered under before `$build` gathers it.
///
/// `$`-prefixed, which no ZDeceptron identifier can be, so a program
/// cannot name one and cannot shadow one.
#[cfg(feature = "evaluate")]
fn global_name(capability: &str) -> String {
    format!("$build${capability}")
}

/// Turn a capability's answer into a JavaScript value, or into a thrown
/// error carrying the refusal verbatim.
#[cfg(feature = "evaluate")]
fn provided(answer: Result<Provided, String>, context: &mut Context) -> JsResult<JsValue> {
    match answer {
        Ok(Provided::Text(text)) | Ok(Provided::Markup(text)) => {
            Ok(JsValue::from(js_string!(text.as_str())))
        }
        Ok(Provided::List(items)) => {
            let values: Vec<JsValue> = items
                .iter()
                .map(|item| JsValue::from(js_string!(item.as_str())))
                .collect();
            Ok(JsArray::from_iter(values, context).into())
        }
        Err(refusal) => Err(JsNativeError::typ().with_message(refusal).into()),
    }
}

/// A JavaScript sandbox the compiler owns, for running the code it just
/// emitted — spec §17.4.8.
///
/// This is the crate's second job, stated in its own module doc: verifying
/// ZDeceptron must not require a JavaScript toolchain. Build-time
/// evaluation is the same requirement pointed at the user rather than at
/// CI. `zdc build` therefore evaluates a `static` signal **in process**,
/// and a developer who uses the fourth placement still installs one binary
/// and nothing else.
#[cfg(feature = "evaluate")]
pub struct Sandbox {
    context: Context,
}

#[cfg(feature = "evaluate")]
impl Default for Sandbox {
    fn default() -> Sandbox {
        Sandbox::new()
    }
}

#[cfg(feature = "evaluate")]
impl Sandbox {
    pub fn new() -> Sandbox {
        let mut context = Context::default();
        context
            .runtime_limits_mut()
            .set_loop_iteration_limit(LOOP_ITERATION_BUDGET);
        Sandbox { context }
    }

    /// Evaluate a module in the sandbox, keeping its bindings for later
    /// questions.
    ///
    /// `export` is stripped rather than honoured: the engine's module
    /// loader wants a filesystem resolver, and a module evaluated as a
    /// script leaves its top-level `const`s where a following `eval` can
    /// see them — which is exactly the interface wanted here.
    pub fn load(&mut self, module: &str) -> Result<(), RuntimeError> {
        let script = strip_exports(module);
        self.context
            .eval(Source::from_bytes(script.as_bytes()))
            .map(|_| ())
            .map_err(RuntimeError::from)
    }

    /// Install the capabilities the code being run may ask the compiler
    /// for, as `$build.<name>(argument)`.
    ///
    /// **This is the whole of the build-time FFI, and its shape is the
    /// argument for it.** A capability is a Rust function pointer with a
    /// fixed signature, resolved against `root` before it is answered.
    /// Nothing is imported, nothing is resolved from a registry, and
    /// nothing outside `root` is reachable — which a module loader could
    /// promise none of.
    ///
    /// `root` is passed to each answer rather than baked into it so the
    /// sandbox boundary is one value, checked in one place, and visible in
    /// every capability's signature.
    pub fn provide(
        &mut self,
        root: &Path,
        capabilities: &[Capability],
    ) -> Result<(), RuntimeError> {
        for capability in capabilities {
            let answer = capability.answer;
            self.context
                .register_global_builtin_callable(
                    js_string!(global_name(capability.name).as_str()),
                    1,
                    NativeFunction::from_copy_closure_with_captures(
                        move |_this, args, root: &PathBuf, context| {
                            let argument = match args.first() {
                                Some(value) => value.to_string(context)?.to_std_string_escaped(),
                                None => {
                                    return Err(JsNativeError::typ()
                                        .with_message("a capability takes one argument")
                                        .into())
                                }
                            };
                            provided(answer(root, &argument), context)
                        },
                        root.to_path_buf(),
                    ),
                )
                .map_err(RuntimeError::from)?;
        }

        // One object, so generated code spells a capability the same way
        // the language does: `build read x` becomes `$build.read(x)`.
        let fields: Vec<String> = capabilities
            .iter()
            .map(|capability| {
                format!(
                    "  {}: {}",
                    capability.name,
                    global_name(capability.name).as_str()
                )
            })
            .collect();
        self.load(&format!(
            "const $build = {{\n{},\n}};\n",
            fields.join(",\n")
        ))
    }

    /// Evaluate an expression and return its value as text.
    ///
    /// `String(value)`, not the engine's debug rendering: a string comes
    /// back as itself, so a caller that asked for `JSON.stringify(x)` gets
    /// the JSON and a caller that asked for a file's contents gets the
    /// contents. There is no framing anywhere in this interface, because
    /// one question returns one answer.
    pub fn text(&mut self, expression: &str) -> Result<String, RuntimeError> {
        let value = self
            .context
            .eval(Source::from_bytes(expression.as_bytes()))
            .map_err(RuntimeError::from)?;
        let text = value
            .to_string(&mut self.context)
            .map_err(RuntimeError::from)?;
        Ok(text.to_std_string_escaped())
    }
}

/// Evaluate `script` with the reactivity core already in scope.
///
/// The core is inlined rather than imported: the engine's module loader
/// wants a filesystem resolver, and the point here is to exercise the
/// exact source that ships, not to test a module loader. `export` is
/// stripped so the same file serves both purposes without a build step.
#[cfg(feature = "evaluate")]
pub fn eval_with_signals(script: &str) -> Result<String, RuntimeError> {
    let mut context = Context::default();
    let core = strip_exports(SIGNAL_JS);

    context
        .eval(Source::from_bytes(core.as_bytes()))
        .map_err(RuntimeError::from)?;

    let value = context
        .eval(Source::from_bytes(script.as_bytes()))
        .map_err(RuntimeError::from)?;

    Ok(value.display().to_string())
}

/// Remove ES module syntax so a module can be evaluated as a script.
///
/// Only leading `export ` is removed. The runtime has no imports between
/// `signal.js` and anything else, which is deliberate — the reactivity
/// core has no dependencies at all, so it can be evaluated in isolation.
#[cfg(feature = "evaluate")]
fn strip_exports(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.strip_prefix("export ") {
            Some(rest) => rest,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_sources_are_not_empty() {
        assert!(SIGNAL_JS.contains("export function signal"));
        assert!(DOM_JS.contains("export function el"));
        assert!(FOREIGN_JS.contains("export function foreign"));
        assert!(MARKUP_JS.contains("export function markup"));
        assert!(MARKUP_JS.contains("export function bindMarkup"));
        // The render path moved out of `dom.js` whole. `template()` still
        // assigns `innerHTML` — parsing one static string per region is
        // what template cloning *is* — so the property is the wrong thing
        // to look for; the exported entry points are the right one.
        assert!(!DOM_JS.contains("export function markup("));
        assert!(!DOM_JS.contains("export function bindMarkup("));
        assert!(RPC_JS.contains("export function remoteCell"));
        assert!(STORE_JS.contains("export function subscribe"));
        assert!(WIRE_JS.contains("export function stringify"));
        assert!(DOM_JS.contains("export function template"));
        assert!(ELEMENTS_JS.contains("export function Column"));
        assert!(BASE_CSS.contains(".zd-col"));
    }

    /// A release build carries no assertion, and a development build does.
    #[test]
    fn a_release_build_drops_the_dev_blocks_a_development_build_keeps() {
        let source = "keep one\n  // $dev\n  throw new Error('x');\n  // $end\nkeep two\n";
        assert_eq!(
            for_mode_str(source, Mode::Release),
            "keep one\nkeep two\n",
            "a release build ships the assertion"
        );
        assert_eq!(
            for_mode_str(source, Mode::Development),
            source,
            "a development build dropped one"
        );
    }

    /// The stripped text is a subsequence of the lines a developer reads.
    ///
    /// This is what makes the two builds one source rather than two: a
    /// marker can only remove lines, so no line can differ between them.
    #[test]
    fn stripping_only_ever_removes_whole_lines() {
        let mut checked = 0;
        for (name, source) in MODULES {
            let release = strip_dev_blocks(source);
            let mut development = source.lines();
            for line in release.lines() {
                checked += 1;
                assert!(
                    development.any(|written| written == line),
                    "{name}: the release build has a line the development build does not: {line}"
                );
            }
        }
        // The runtime is thousands of lines; a loop that checked a handful
        // of them would be a loop that had stopped finding the modules.
        assert!(
            MODULES.len() >= 8 && checked > 2_000,
            "{checked} lines compared across {} modules",
            MODULES.len()
        );
    }

    /// What minification takes off the runtime a reader downloads — #135.
    ///
    /// The number is asserted, not just the direction. "Smaller" is
    /// satisfied by one byte, and a minifier that had quietly stopped
    /// finding comments would still satisfy it; the claim in
    /// `BENCHMARKS.md` and in `minify.rs`'s own doc comment is that the
    /// runtime loses most of its bytes, so most of its bytes is what this
    /// checks.
    ///
    /// Bounded on both sides. A release build that came out *drastically*
    /// smaller than this would mean the scanner had eaten code — which is
    /// the failure mode a size gate on its own would applaud.
    #[test]
    fn a_release_build_is_minified_and_this_is_what_it_saves() {
        let mut source_bytes = 0;
        let mut shipped_bytes = 0;
        let mut checked = 0;
        for (name, module) in MODULES {
            let release = for_mode(module, Mode::Release);
            assert!(
                release.len() < module.len(),
                "{name} is no smaller as a release build than as source"
            );
            assert_eq!(
                for_mode(module, Mode::Development).as_ref(),
                *module,
                "{name}: a development build is the source, unchanged"
            );
            source_bytes += module.len();
            shipped_bytes += release.len();
            checked += 1;
        }
        assert_eq!(checked, MODULES.len(), "a module went unmeasured");
        assert!(
            MODULES.len() >= 8,
            "only {} modules surveyed; the list has stopped naming the runtime",
            MODULES.len()
        );
        assert!(
            shipped_bytes * 2 < source_bytes,
            "the runtime ships {shipped_bytes} bytes against {source_bytes} of \
             source. Under half is the claim #135 was closed on; above it, \
             either the comments have gone or the minifier has."
        );
        assert!(
            shipped_bytes * 10 > source_bytes,
            "the runtime ships {shipped_bytes} bytes against {source_bytes} of \
             source, which is under a tenth. Comments and indentation do not \
             account for that, so the scanner has eaten code — the one failure \
             a size measurement on its own would report as a success."
        );
    }

    /// Minification renames nothing, so every export is still spelled the
    /// way the module that imports it spells it.
    ///
    /// This is the property that makes the safe subset safe, and it is
    /// what a mangling minifier would give up. It is checked against the
    /// release build rather than the source, because the release build is
    /// the one a browser resolves the import against.
    #[test]
    fn a_minified_module_keeps_the_names_its_importers_use() {
        let names = [
            (SIGNAL_JS, "export function signal("),
            (DOM_JS, "export function template("),
            (LIST_JS, "export function each("),
            (WIRE_JS, "export function stringify("),
            (RPC_JS, "export function remoteCell("),
        ];
        for (module, exported) in names {
            let release = for_mode(module, Mode::Release);
            assert!(
                release.contains(exported),
                "a release build lost `{exported}`, so an importer of it \
                 would fail to resolve"
            );
        }
    }

    /// Every marker in every module is matched, and none is nested.
    ///
    /// Without this an unclosed `// $dev` would silently delete the rest of
    /// a module from every release build, which is the worst failure this
    /// mechanism could have: it compiles, it ships, and the missing code is
    /// whatever came after the mistake.
    #[test]
    fn dev_blocks_are_balanced() {
        let mut blocks = 0;
        for (name, source) in MODULES {
            let mut inside = false;
            for (number, line) in source.lines().enumerate() {
                match line.trim() {
                    DEV_OPEN => {
                        assert!(!inside, "{name}:{}: a nested `{DEV_OPEN}`", number + 1);
                        inside = true;
                        blocks += 1;
                    }
                    DEV_CLOSE => {
                        assert!(inside, "{name}:{}: a stray `{DEV_CLOSE}`", number + 1);
                        inside = false;
                    }
                    _ => {}
                }
            }
            assert!(!inside, "{name}: a `{DEV_OPEN}` block was never closed");
        }
        assert!(
            blocks >= 2,
            "only {blocks} dev blocks in the whole runtime; the mechanism is \
             not carrying any assertions, so nothing it claims is tested"
        );
    }

    fn for_mode_str(source: &str, mode: Mode) -> String {
        match mode {
            Mode::Development => source.to_string(),
            Mode::Release => strip_dev_blocks(source),
        }
    }

    /// The tests below this line all need an engine to run. They are
    /// gated on the same feature the engine is, so `--no-default-features`
    /// still runs the ones that cover the half of the crate that remains
    /// — rather than compiling nothing and calling it a pass.
    #[cfg(feature = "evaluate")]
    #[test]
    fn stripping_exports_leaves_the_declaration() {
        assert_eq!(
            strip_exports("export function signal(x) {}"),
            "function signal(x) {}"
        );
        assert_eq!(strip_exports("  indented stays"), "  indented stays");
    }

    #[cfg(feature = "evaluate")]
    #[test]
    fn a_provided_capability_answers_the_code_it_is_running() {
        fn shout(root: &Path, argument: &str) -> Result<Provided, String> {
            Ok(Provided::Text(format!(
                "{}/{}",
                root.display(),
                argument.to_uppercase()
            )))
        }
        fn twice(_root: &Path, argument: &str) -> Result<Provided, String> {
            Ok(Provided::List(vec![
                argument.to_string(),
                argument.to_string(),
            ]))
        }

        let mut sandbox = Sandbox::new();
        sandbox
            .provide(
                Path::new("/project"),
                &[
                    Capability {
                        name: "shout",
                        answer: shout,
                    },
                    Capability {
                        name: "twice",
                        answer: twice,
                    },
                ],
            )
            .expect("capabilities install");

        assert_eq!(
            sandbox.text("$build.shout(\"hi\")").expect("answers"),
            "/project/HI"
        );
        assert_eq!(
            sandbox
                .text("$build.twice(\"a\").join(\",\")")
                .expect("answers"),
            "a,a"
        );
    }

    /// A refusal is a thrown error, so it stops the build rather than
    /// becoming a value the program goes on to inline.
    #[cfg(feature = "evaluate")]
    #[test]
    fn a_refused_capability_stops_the_evaluation() {
        fn always_refuses(_root: &Path, _argument: &str) -> Result<Provided, String> {
            Err("no".to_string())
        }

        let mut sandbox = Sandbox::new();
        sandbox
            .provide(
                Path::new("/project"),
                &[Capability {
                    name: "nope",
                    answer: always_refuses,
                }],
            )
            .expect("capabilities install");

        let error = sandbox.text("$build.nope(\"x\")").expect_err("must refuse");
        assert!(error.message.contains("no"), "{error}");
        assert!(!error.budget_exceeded);
    }

    #[cfg(feature = "evaluate")]
    #[test]
    fn a_signal_round_trips_through_the_engine() {
        let out = eval_with_signals(
            r#"
            const [get, set] = signal(1);
            set(41);
            get() + 1
            "#,
        )
        .expect("evaluates");
        assert_eq!(out, "42");
    }
}
