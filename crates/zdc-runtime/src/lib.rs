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

use boa_engine::{Context, JsError, Source};

/// The reactivity core: signals, derived values, effects, batching.
pub const SIGNAL_JS: &str = include_str!("../../../runtime/signal.js");

/// DOM rendering. Requires a document, so it is embedded for shipping
/// rather than for evaluation here.
pub const DOM_JS: &str = include_str!("../../../runtime/dom.js");

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
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RuntimeError {}

impl From<JsError> for RuntimeError {
    fn from(error: JsError) -> Self {
        RuntimeError {
            message: error.to_string(),
        }
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
