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
#![forbid(unsafe_code)]

use boa_engine::{Context, JsError, JsNativeErrorKind, Source};

/// The reactivity core: signals, derived values, effects, batching.
pub const SIGNAL_JS: &str = include_str!("../../../runtime/signal.js");

/// DOM rendering. Requires a document, so it is embedded for shipping
/// rather than for evaluation here.
pub const DOM_JS: &str = include_str!("../../../runtime/dom.js");

/// The client half of the derived boundary: `$remote` and `$call`.
///
/// A bundle links against this only when the split found a crossing, so a
/// client-only program still ships nothing it does not use (§16.3.1).
pub const RPC_JS: &str = include_str!("../../../runtime/rpc.js");

/// The wire format: how a ZD value survives JSON.
///
/// Its own module because three separate things encode and decode with it
/// — the browser, the platform adapter, and the live-sync stream — and a
/// second copy of the rules is how they come to disagree.
pub const WIRE_JS: &str = include_str!("../../../runtime/wire.js");

/// Live sync for `durable` placement, and the transport seam it needs.
///
/// Shipped only when the split found a durable key. It imports `rpc.js`,
/// which a program with a crossing already has.
pub const STORE_JS: &str = include_str!("../../../runtime/store.js");

/// The built-in view elements.
pub const ELEMENTS_JS: &str = include_str!("../../../runtime/elements.js");

/// The base styling of the built-in elements, as classes.
///
/// Spec §16.2 R6: `Column` and `Row` carry `zd-col`/`zd-row` rather than an
/// inline style object, so the declarations have to ship somewhere. This is
/// the base layer of the `styles.css` a build emits.
pub const BASE_CSS: &str = include_str!("../../../runtime/base.css");

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
const LOOP_ITERATION_BUDGET: u64 = 10_000_000;

/// A JavaScript sandbox the compiler owns, for running the code it just
/// emitted — spec §17.4.8.
///
/// This is the crate's second job, stated in its own module doc: verifying
/// ZDeceptron must not require a JavaScript toolchain. Build-time
/// evaluation is the same requirement pointed at the user rather than at
/// CI. `zdc build` therefore evaluates a `static` signal **in process**,
/// and a developer who uses the fourth placement still installs one binary
/// and nothing else.
pub struct Sandbox {
    context: Context,
}

impl Default for Sandbox {
    fn default() -> Sandbox {
        Sandbox::new()
    }
}

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
        assert!(RPC_JS.contains("export function remoteCell"));
        assert!(STORE_JS.contains("export function subscribe"));
        assert!(WIRE_JS.contains("export function stringify"));
        assert!(DOM_JS.contains("export function template"));
        assert!(ELEMENTS_JS.contains("export function Column"));
        assert!(BASE_CSS.contains(".zd-col"));
    }

    #[test]
    fn stripping_exports_leaves_the_declaration() {
        assert_eq!(
            strip_exports("export function signal(x) {}"),
            "function signal(x) {}"
        );
        assert_eq!(strip_exports("  indented stays"), "  indented stays");
    }

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
