//! Running the `BUILD` root on the build host — spec §17.4.8.
//!
//! §17.4.8 replaced a Rust interpreter with "execute the module, exactly
//! like any other root", and then said the build host needs a JavaScript
//! runtime for it. The first half is right and the second half is not: the
//! compiler already carries a JavaScript engine, `zdc-runtime`'s, whose
//! whole reason for existing is that **needing Node to build ZDeceptron
//! would be the first crack in the claim that a developer installs one
//! binary and nothing else**.
//!
//! So the build root is evaluated **in process**, in a sandbox the
//! compiler owns. `zdc build` spawns nothing. A developer who reaches for
//! the fourth placement — and on the milestone-7 target every page does —
//! installs exactly what they installed before, which is `zdc`.
//!
//! Two consequences follow, and both are improvements:
//!
//! * **Non-termination is bounded rather than timed.** §17.4.8 wanted a
//!   wall clock because there is nothing to meter in someone else's
//!   process. In an engine the compiler owns there is. The bound is
//!   deterministic, so a build that fails does so on every machine —
//!   §14A.4 cannot tolerate a failure that depends on how busy the host
//!   happened to be.
//! * **A generated file never touches the filesystem here.** Contents come
//!   back as strings and the caller writes them where it chose, so there is
//!   no path at all by which a build could write somewhere the compiler did
//!   not name. E0316's check on the declared path stays, as the outer of
//!   the two.

use std::collections::BTreeMap;
use std::path::Path;

use zdc_lexer::Span;
use zdc_runtime::Sandbox;

use crate::build::{BuildModule, Claim};
use crate::capability;
use crate::js;

/// Why a `static` value could not be computed.
///
/// Carries a code because these are diagnostics like any other, and a
/// programmer who searches for `E9` should find the one that explains that
/// build-time evaluation has to terminate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationError {
    pub code: &'static str,
    pub message: String,
    pub help: String,
}

impl EvaluationError {
    /// One block of text, the way a file-level diagnostic wants it.
    pub fn report(&self) -> String {
        format!("[{}] {}\n  help: {}", self.code, self.message, self.help)
    }
}

/// What one run of the build root produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evaluated {
    /// Every `static` value, by source name, as JSON — for inlining.
    pub values: BTreeMap<String, String>,
    /// Every generated file, by its path in the bundle — §14C.3b.
    ///
    /// Returned rather than written where they belong, because the caller
    /// owns the output: `zdc build` writes them into `--out`, and `zdc dev`
    /// serves them from memory without writing anything at all.
    pub files: BTreeMap<String, String>,
}

/// Refuses the values that have no literal form, before one is asked for.
///
/// A `Map` stringifies to `{}` and an absent value stringifies to the word
/// `undefined`; either would inline something that is quietly not what the
/// program computed, which is worse than a refusal.
const GUARD: &str = r#"const $inlinable = (key, value) => {
  if (value instanceof Map || value instanceof Set) {
    throw new Error("holds a Map or a Set, which has no literal form to inline");
  }
  if (typeof value === "function" || typeof value === "undefined") {
    throw new Error("did not produce a value");
  }
  return value;
};
"#;

/// Compute every `static` value, returning them by source name as JSON.
///
/// `directory` is where the program's source file lives, and it is the
/// **project directory**: the whole of what a build may read. Every
/// capability in [`crate::capability`] is resolved against it before it is
/// answered, so a build reads the project it is building and nothing else.
/// The stack a build-host evaluation runs on.
///
/// Windows gives a process's main thread one megabyte where Unix gives
/// eight, and evaluating a `static` value or a claim means *running the
/// program*: a ZDeceptron function that recurses — which is how a program
/// with no `fold` loops at all — recurses in the interpreter too, several
/// engine frames deep per call. `examples/sorting.test.zd` sorts twenty
/// elements and overflowed there while passing on every other platform.
///
/// Sixteen megabytes, which is twice what Unix hands out, so the platforms
/// agree about which programs are too deep instead of disagreeing by a
/// factor of eight.
const EVALUATION_STACK: usize = 16 * 1024 * 1024;

/// Run `work` on a thread with [`EVALUATION_STACK`] to stand on.
///
/// Prevented rather than caught: a stack overflow aborts the process, so
/// there is no error to return and nothing to report. Both entry points go
/// through here, because `zdc build` evaluates the same recursive programs
/// `zdc test` does — a file whose expectations are `static` is evaluated by
/// both, and fixing only one of them moves the crash rather than removing
/// it.
fn on_a_deep_stack<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(EVALUATION_STACK)
            .spawn_scoped(scope, work)
            .expect("a thread to evaluate on")
            .join()
            .expect("evaluation must not panic")
    })
}

pub fn evaluate(module: &BuildModule, directory: &Path) -> Result<Evaluated, EvaluationError> {
    on_a_deep_stack(|| evaluate_on_this_thread(module, directory))
}

fn evaluate_on_this_thread(
    module: &BuildModule,
    directory: &Path,
) -> Result<Evaluated, EvaluationError> {
    let mut sandbox = Sandbox::new();
    // Capabilities are installed **before** the module runs, because the
    // module's top-level `const`s are where a `static` value is computed.
    sandbox
        .provide(directory, &capability::all())
        .map_err(|error| failure(module, error))?;
    sandbox
        .load(&module.source)
        .map_err(|error| failure(module, error))?;
    sandbox
        .load(GUARD)
        .map_err(|error| failure(module, error))?;

    // One question, one answer, and no framing anywhere. Asking per name is
    // what lets a file's contents be any text at all, including the tabs
    // and newlines a delimited protocol would have had to escape.
    let mut values = BTreeMap::new();
    for name in &module.statics {
        let json = sandbox
            .text(&format!(
                "JSON.stringify($values[{}], $inlinable)",
                js::string(name)
            ))
            .map_err(|error| uncomputable(name, error))?;
        values.insert(name.clone(), json);
    }

    // Read back by the path the program declared, not by asking the module
    // what it wrote: `$files`' keys are the contract, so there is no set of
    // keys for the two sides to disagree about.
    let mut files = BTreeMap::new();
    for (path, name) in &module.emits {
        let contents = sandbox
            .text(&format!("$files[{}]", js::string(path)))
            .map_err(|error| uncomputable(name, error))?;
        files.insert(path.clone(), contents);
    }

    Ok(Evaluated { values, files })
}

/// What one `test` declaration came to — issue #169.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The sentence the test asserts, as written.
    pub claim: String,
    /// The `expect` clause, for the caret.
    pub span: Span,
    pub verdict: ClaimVerdict,
}

/// The three things that can happen to a claim.
///
/// `ClaimVerdict` rather than `Verdict` because [`zdc_graph::Verdict`] is
/// already the information-flow pass's answer, and two types called
/// `Verdict` in one crate is a name a reader has to disambiguate every
/// time they meet it.
///
/// Three and not two. A claim that *could not be evaluated* — a budget
/// exhausted by a runaway recursion, a capability refused, a `foreign`
/// that threw — is not a false claim, and reporting it as one would tell
/// the reader the program is wrong when what is wrong is the test. They
/// get different codes for the same reason `E9`, `E10` and `E11` are three
/// codes and not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimVerdict {
    Held,
    Broken(Broken),
    Unevaluable(EvaluationError),
}

/// A claim the program contradicted — issue #169.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken {
    /// Carried rather than written by the renderer, exactly as
    /// [`zdc_graph::GraphError`] carries its own. The code and the site
    /// that raises it belong in one file, so `zdc explain`'s coverage gate
    /// can enumerate the codes from the source that produces them.
    pub code: &'static str,
    pub claim: String,
    pub span: Span,
    /// What each side of a top-level `is` came to, when the expectation
    /// had two sides. `None` when it did not — `a and b`, `xs contains y`,
    /// a call returning a `Truth` — because there is no pair to show and
    /// inventing one would point the reader at the wrong values.
    pub sides: Option<(String, String)>,
}

/// Rendering a value for a human, in the report.
///
/// **Not `JSON.stringify`.** The values a claim compares include the ones
/// §17.4.8's `$inlinable` guard *refuses* — a `Map`, a `Set` — and a
/// report that threw rather than showing the value would fail on exactly
/// the comparison the reader most needs help with. This renders everything
/// and refuses nothing, which it can afford to do because the result is
/// printed rather than inlined into a program.
///
/// Text is quoted so that `"1"` and `1` are distinguishable, which is the
/// single most common thing to be confused about when a comparison
/// surprises someone.
const SHOW: &str = r#"const $show = (value) => {
  if (typeof value === "string") return JSON.stringify(value);
  if (value instanceof Map) {
    return "[" + [...value].map(([k, v]) => $show(k) + " to " + $show(v)).join(", ") + "]";
  }
  if (value instanceof Set) return "[" + [...value].map($show).join(", ") + "]";
  if (Array.isArray(value)) return "[" + value.map($show).join(", ") + "]";
  if (value === null || value === undefined) return "nothing";
  if (typeof value === "boolean") return value ? "yes" : "no";
  if (typeof value === "object") {
    return "(" + Object.keys(value).map((k) => k + " is " + $show(value[k])).join(", ") + ")";
  }
  return String(value);
};
"#;

/// Evaluate every claim the build root carries, in declaration order.
///
/// # Why this reuses the build root rather than inventing a runner
///
/// §17.4.8 already had to answer *how do you run this program's own code
/// with no browser and no Node*, and the answer — print an ordinary
/// JavaScript module, evaluate it in the engine the compiler already links
/// — is the same answer a test runner needs. A second mechanism would be a
/// second implementation of every primitive, checked by nothing, and it
/// would disagree with the shipped one exactly where a test is supposed to
/// notice.
///
/// So a claim is checked by **running the code the compiler emits**, which
/// is the property that makes the result worth anything: what passed here
/// is what the browser will run.
///
/// One expectation is one question. A claim that throws does not abandon
/// the run — the remaining claims are still asked, because a suite that
/// stops at the first problem tells the reader about one of their four
/// mistakes.
pub fn run_tests(module: &BuildModule, directory: &Path) -> Result<Vec<Outcome>, EvaluationError> {
    on_a_deep_stack(|| run_tests_on_this_thread(module, directory))
}

fn run_tests_on_this_thread(
    module: &BuildModule,
    directory: &Path,
) -> Result<Vec<Outcome>, EvaluationError> {
    if module.tests.is_empty() {
        return Ok(Vec::new());
    }
    let mut sandbox = Sandbox::new();
    sandbox
        .provide(directory, &capability::all())
        .map_err(|error| failure(module, error))?;
    sandbox
        .load(&module.source)
        .map_err(|error| failure(module, error))?;
    sandbox.load(SHOW).map_err(|error| failure(module, error))?;

    let mut outcomes = Vec::new();
    for (index, claim) in module.tests.iter().enumerate() {
        outcomes.push(Outcome {
            claim: claim.claim.clone(),
            span: claim.span,
            verdict: verdict_of(&mut sandbox, index, claim),
        });
    }
    Ok(outcomes)
}

/// Ask the sandbox one claim, and read back what it said.
fn verdict_of(sandbox: &mut Sandbox, index: usize, claim: &Claim) -> ClaimVerdict {
    // `=== true`, not a truthiness test. The expectation is typechecked as
    // a `Truth`, so anything else is the compiler having emitted something
    // it should not have, and a truthy `"no"` passing silently is the one
    // failure this whole feature exists to prevent.
    let held = match sandbox.text(&format!("String($tests[{index}].run() === true)")) {
        Ok(answer) => answer,
        Err(error) => return ClaimVerdict::Unevaluable(unevaluable(&claim.claim, error)),
    };
    if held == "true" {
        return ClaimVerdict::Held;
    }
    ClaimVerdict::Broken(Broken {
        code: "E-TEST-01",
        claim: claim.claim.clone(),
        span: claim.span,
        sides: claim.comparison.then(|| sides(sandbox, index)).flatten(),
    })
}

/// The two rendered operands of a broken comparison.
///
/// A side that cannot be rendered gives up on the *pair* rather than
/// showing one half: "left is 3" with no right is a fact the reader cannot
/// use, and it reads as though the right side were missing from their
/// program.
fn sides(sandbox: &mut Sandbox, index: usize) -> Option<(String, String)> {
    let left = sandbox
        .text(&format!("$show($tests[{index}].left())"))
        .ok()?;
    let right = sandbox
        .text(&format!("$show($tests[{index}].right())"))
        .ok()?;
    Some((left, right))
}

/// A claim the runner could not decide either way — issue #169.
fn unevaluable(claim: &str, error: zdc_runtime::RuntimeError) -> EvaluationError {
    if error.budget_exceeded {
        return EvaluationError {
            code: "E-TEST-02",
            message: format!(
                "the claim `{claim}` did more work than one expectation is allowed to, so it is \
                 neither held nor broken ({error})."
            ),
            help: "The bound is on loops and recursion and is the same on every machine, so a \
                   claim that stops here stops everywhere. An expectation that does not \
                   terminate is usually a claim about a function that does not either (spec \
                   §17.4.8)."
                .to_string(),
        };
    }
    if let Some(reason) = refusal(&error) {
        return EvaluationError {
            code: "E-TEST-02",
            message: format!("the claim `{claim}` was refused: {reason}."),
            help: "A claim is evaluated at build time, so it reads the project directory it was \
                   pointed at and nothing else — the same sandbox `zdc build` gives a `static` \
                   signal."
                .to_string(),
        };
    }
    EvaluationError {
        code: "E-TEST-02",
        message: format!("the claim `{claim}` could not be decided: {error}."),
        help: "The expectation stopped before it produced a `yes` or a `no`, so the claim is \
               neither held nor broken. Whatever it called failed; fix that first."
            .to_string(),
    }
}

/// The build root would not even load, so every `static` in the program is
/// named: none of them has a value.
fn failure(module: &BuildModule, error: zdc_runtime::RuntimeError) -> EvaluationError {
    let named = module.statics.join("`, `");
    if error.budget_exceeded {
        return budget(&named, error);
    }
    // A `static` value is a top-level `const`, so a capability is answered
    // while the module is loading. A refusal arrives here rather than at
    // the question that follows.
    if let Some(reason) = refusal(&error) {
        return refused(&named, &reason);
    }
    EvaluationError {
        code: "E10",
        message: format!("the build host could not compute `{named}`: {error}"),
        help: HELP.to_string(),
    }
}

fn uncomputable(name: &str, error: zdc_runtime::RuntimeError) -> EvaluationError {
    if error.budget_exceeded {
        return budget(name, error);
    }
    if let Some(reason) = refusal(&error) {
        return refused(name, &reason);
    }
    EvaluationError {
        code: "E10",
        message: format!("the build host could not compute `{name}`: {error}"),
        help: HELP.to_string(),
    }
}

/// The refusal a capability threw, if that is what stopped the build.
///
/// A capability's only channel back through the engine is the message on
/// the error it throws, so refusals are marked on the way out and read
/// back here. A program that threw for its own reasons has no marker and
/// stays E10 — the two are different mistakes.
fn refusal(error: &zdc_runtime::RuntimeError) -> Option<String> {
    error
        .message
        .split_once(capability::REFUSED)
        .map(|(_, reason)| reason.to_string())
}

/// §14C.3b's read half, bounded the way its write half already is.
fn refused(name: &str, reason: &str) -> EvaluationError {
    EvaluationError {
        code: "E11",
        message: format!("computing `{name}` was refused: {reason}."),
        help: "A build reads the project directory it was pointed at. An absolute path, a path \
               climbing out with `..`, and a symbolic link resolving outside it are each refused \
               — as the *resolved* path, so a link cannot launder one. This is the read-side of \
               the rule E0316 already applies to a `static` file's declared output path."
            .to_string(),
    }
}

/// §17.4.8's E9, as a bound rather than a clock.
fn budget(name: &str, error: zdc_runtime::RuntimeError) -> EvaluationError {
    EvaluationError {
        code: "E9",
        message: format!(
            "evaluating `{name}` did more work than a build is allowed to; a `static` value is \
             computed at build time, so its computation must terminate ({error})."
        ),
        help: "The bound is on loops and recursion, and it is the same on every machine, so a \
               build that fails here fails everywhere (spec §17.4.8)."
            .to_string(),
    }
}

const HELP: &str = "A `static` signal runs once at build time, in a server environment (spec \
                    §14G.1.5). Everything it reads has to exist there.";
