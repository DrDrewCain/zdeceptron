//! Running the `BUILD` root on the build host — spec §17.4.8.
//!
//! §17.4.8 replaced a Rust interpreter with "execute the module, exactly
//! like any other root". This is that execution, and it is deliberately
//! small: write the printed module out, run it under the host's JavaScript
//! runtime, read back one line per `static` signal.
//!
//! **Non-termination is the host's problem.** §17.4.8 gives it a wall-clock
//! budget rather than a fuel counter, because there is no interpreter to
//! meter — `E9` below is that budget.
//!
//! **The named cost.** A program that uses `static` needs a JavaScript
//! runtime on the build host. A program that does not never reaches this
//! module, so `hello.zd` through `todo.zd` still build without one.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::build::BuildModule;

/// §17.4.8's wall-clock budget.
const BUDGET: Duration = Duration::from_secs(30);

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

/// The driver. Printed beside the build root rather than appended to it, so
/// the module stays an ordinary module: §17.4.8 says the `BUILD` root is
/// emitted "exactly like any other root", and a root with a `process.stdout`
/// call in it would not be.
///
/// One line per value, `name`, tab, JSON. `JSON.stringify` escapes every
/// control character, so neither a tab nor a newline can occur inside a
/// field and the framing needs no quoting of its own.
///
/// Generated files (§14C.3b) go the other way: their contents can be any
/// text at all, so they are written to a directory the compiler names and
/// read back from it, rather than framed onto a pipe. The directory is
/// `argv[2]`, and every path in `$files` was checked at compile time to be
/// relative and non-climbing, so nothing here can write outside it.
const DRIVER: &str = r#"import { $values, $files } from "./build.mjs";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const inlinable = (key, value) => {
  if (value instanceof Map || value instanceof Set) {
    throw new Error(`\`${key}\` holds a Map or a Set, which has no literal form to inline.`);
  }
  if (typeof value === "function" || typeof value === "undefined") {
    throw new Error(`\`${key}\` did not produce a value.`);
  }
  return value;
};

let out = "";
for (const key of Object.keys($values)) {
  out += key + "\t" + JSON.stringify($values[key], inlinable) + "\n";
}

const into = process.argv[2];
for (const path of Object.keys($files)) {
  const contents = $files[path];
  if (typeof contents !== "string") {
    throw new Error(`\`${path}\` was written from a value that is not text.`);
  }
  const target = join(into, path);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, contents, "utf8");
}

process.stdout.write(out);
"#;

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

/// Compute every `static` value, returning them by source name as JSON.
///
/// `directory` is the directory the program's source file lives in, and it
/// becomes the process's working directory: a program that reads
/// `"content"` at build time means the `content` beside itself, not the one
/// beside whatever shell invoked the compiler.
pub fn evaluate(module: &BuildModule, directory: &Path) -> Result<Evaluated, EvaluationError> {
    let workspace = scratch()?;
    write(&workspace.join("build.mjs"), &module.source)?;
    write(&workspace.join("driver.mjs"), DRIVER)?;
    let emitted_into = workspace.join("emitted");
    std::fs::create_dir_all(&emitted_into).map_err(|e| unwritable(&emitted_into, e))?;

    let out_path = workspace.join("values.txt");
    let err_path = workspace.join("errors.txt");
    // Redirected to files rather than piped: a piped child that fills the
    // pipe buffer blocks until it is drained, and the loop below is not
    // draining it — it is watching the clock.
    let out_file = std::fs::File::create(&out_path).map_err(|e| unwritable(&out_path, e))?;
    let err_file = std::fs::File::create(&err_path).map_err(|e| unwritable(&err_path, e))?;

    let mut child = Command::new("node")
        .arg(workspace.join("driver.mjs"))
        .arg(&emitted_into)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .spawn()
        .map_err(missing_runtime)?;

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                return Err(EvaluationError {
                    code: "E9",
                    message: format!(
                        "The build host's JavaScript runtime could not be waited on: {e}"
                    ),
                    help: "Check that `node` is on the path and can be executed.".to_string(),
                })
            }
        }
        if started.elapsed() >= BUDGET {
            let _ = child.kill();
            let _ = child.wait();
            return Err(EvaluationError {
                code: "E9",
                message: format!(
                    "evaluating {} exceeded {} seconds; a `static` value is computed at build \
                     time, so its computation must terminate.",
                    module
                        .statics
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    BUDGET.as_secs()
                ),
                help: "A `static` signal is evaluated once, on the build host. It cannot wait on \
                       anything that only exists at run time (spec §17.4.8)."
                    .to_string(),
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    if !status.success() {
        let details = std::fs::read_to_string(&err_path).unwrap_or_default();
        return Err(EvaluationError {
            code: "E10",
            message: format!(
                "the build host could not compute {}.\n{}",
                module
                    .statics
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                details.trim_end()
            ),
            help: "A `static` signal runs once at build time, in a server environment (spec \
                   §14G.1.5). Everything it reads has to exist there."
                .to_string(),
        });
    }

    let text = std::fs::read_to_string(&out_path).map_err(|e| EvaluationError {
        code: "E10",
        message: format!("the build host's answers could not be read back: {e}"),
        help: "This is a compiler bug; the build root ran and reported success.".to_string(),
    })?;

    // Read back by the path the program declared, not by walking the
    // directory: `$files`' keys are the contract, and a file the build root
    // wrote under some other name is not one this program asked for.
    let mut files = BTreeMap::new();
    for (path, name) in &module.emits {
        let written = emitted_into.join(path);
        let contents = std::fs::read_to_string(&written).map_err(|e| EvaluationError {
            code: "E10",
            message: format!("`{name}` was to be written to `{path}`, and was not: {e}"),
            help: "This is a compiler bug; the build root ran and reported success.".to_string(),
        })?;
        files.insert(path.clone(), contents);
    }
    let _ = std::fs::remove_dir_all(&workspace);

    let mut values = BTreeMap::new();
    for line in text.lines() {
        let Some((name, json)) = line.split_once('\t') else {
            continue;
        };
        values.insert(name.to_string(), json.to_string());
    }

    // Every declared `static` must have an answer. A missing one would
    // otherwise reach `Emitter::reference`, which would report it as
    // "build-time evaluation is not wired up" — true once, and misleading
    // now.
    for name in &module.statics {
        if !values.contains_key(name) {
            return Err(EvaluationError {
                code: "E10",
                message: format!("the build host computed no value for `{name}`."),
                help: "This is a compiler bug; the build root ran and reported success."
                    .to_string(),
            });
        }
    }

    Ok(Evaluated { values, files })
}

fn missing_runtime(error: std::io::Error) -> EvaluationError {
    if error.kind() == std::io::ErrorKind::NotFound {
        return EvaluationError {
            code: "E11",
            message: "this program has `static` state, and computing it needs a JavaScript \
                      runtime on the build host. `node` was not found."
                .to_string(),
            help: "Install Node, or move the state to `client`, `server` or `durable`, none of \
                   which is computed at build time (spec §17.4.8)."
                .to_string(),
        };
    }
    EvaluationError {
        code: "E11",
        message: format!("the build host's JavaScript runtime could not be started: {error}"),
        help: "`static` state is computed by running the build root under `node` (spec §17.4.8)."
            .to_string(),
    }
}

fn unwritable(path: &Path, error: std::io::Error) -> EvaluationError {
    EvaluationError {
        code: "E10",
        message: format!("could not write {}: {error}", path.display()),
        help: "Build-time evaluation needs a writable temporary directory.".to_string(),
    }
}

fn write(path: &Path, contents: &str) -> Result<(), EvaluationError> {
    std::fs::write(path, contents).map_err(|e| unwritable(path, e))
}

/// A fresh directory to run in. Named from the clock and the process, so two
/// concurrent builds — `zdc dev` rebuilding while `zdc build` runs — never
/// hand each other half a module.
fn scratch() -> Result<std::path::PathBuf, EvaluationError> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("zdc-build-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&path).map_err(|e| unwritable(&path, e))?;
    Ok(path)
}
