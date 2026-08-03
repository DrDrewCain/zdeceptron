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

use zdc_runtime::Sandbox;

use crate::build::BuildModule;
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
/// `directory` is where the program's source file lives. Nothing reads it
/// yet — a build root can compute but cannot yet *read*, because reading
/// needs `foreign` (§14E) and there is no `foreign` — and it is taken now
/// so the signature does not change when there is.
pub fn evaluate(module: &BuildModule, directory: &Path) -> Result<Evaluated, EvaluationError> {
    let _ = directory;

    let mut sandbox = Sandbox::new();
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

/// The build root would not even load, so every `static` in the program is
/// named: none of them has a value.
fn failure(module: &BuildModule, error: zdc_runtime::RuntimeError) -> EvaluationError {
    let named = module.statics.join("`, `");
    if error.budget_exceeded {
        return budget(&named, error);
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
    EvaluationError {
        code: "E10",
        message: format!("the build host could not compute `{name}`: {error}"),
        help: HELP.to_string(),
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
