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

use std::path::{Path, PathBuf};

use boa_engine::object::builtins::JsArray;
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
fn global_name(capability: &str) -> String {
    format!("$build${capability}")
}

/// Turn a capability's answer into a JavaScript value, or into a thrown
/// error carrying the refusal verbatim.
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

    #[test]
    fn stripping_exports_leaves_the_declaration() {
        assert_eq!(
            strip_exports("export function signal(x) {}"),
            "function signal(x) {}"
        );
        assert_eq!(strip_exports("  indented stays"), "  indented stays");
    }

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
