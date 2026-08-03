//! What the approach costs as programs get bigger — Swift's question.
//!
//! Swift (SOSP'07) is ZDeceptron's thesis already built: security labels
//! driving automatic client/server partitioning. It won Best Paper and it
//! was abandoned, and the number that says why is **~800 bytes of
//! JavaScript per line of source** — a 6-line null program emitting 73 kB
//! and a 1,094-line application emitting 1.21 MB. The machinery that makes
//! the network boundary invisible ended up in the bundle.
//!
//! That is the documented failure mode of this entire design, and until
//! now ZDeceptron had never been measured against it. This module supplies
//! the measurement, in the same units, plus the three things that would
//! turn a good number today into a bad one later:
//!
//! * whether emitted size grows **linearly** with source size, or worse;
//! * whether tier splitting is really the **product** of the definition set
//!   and the root set that §17.2 describes — routing multiplies the roots,
//!   one per page, so a quadratic here is a finding for the routing work;
//! * how deep a **fold** goes before the host's recursion budget gives out,
//!   since §17.4.9's index recursion is linear in stack depth.
//!
//! Everything here is either an exact byte count or a wall-clock time of
//! *the compiler*, which is Rust. Nothing here times generated JavaScript:
//! see `BENCHMARKS.md` for why the embedded interpreter cannot resolve that
//! question either way.

use std::time::{Duration, Instant};

use crate::sizes::{repository_path, try_compile};

/// Swift's headline: bytes of JavaScript per line of application source.
pub const SWIFT_BYTES_PER_LINE: usize = 800;
/// Swift's null program: 6 lines in, 73 kB of JavaScript out.
pub const SWIFT_NULL_PROGRAM_LINES: usize = 6;
/// Swift's null program, in bytes.
pub const SWIFT_NULL_PROGRAM_JS: usize = 73_000;
/// Swift's largest application, `Shop`.
pub const SWIFT_LARGEST_APP_LINES: usize = 1_094;
/// `Shop`'s emitted JavaScript, in bytes.
pub const SWIFT_LARGEST_APP_JS: usize = 1_210_000;

/// The JavaScript every bundle links against, whatever the program.
///
/// `elements.js` is not in the sum: generated code never imports it
/// (§16.3.1), so it is not shipped. Uncompressed and unminified, because
/// there is no minifier in the pipeline and a projected figure would be a
/// claim about a tool that does not exist.
pub fn runtime_js_bytes() -> usize {
    zdc_runtime::SIGNAL_JS.len() + zdc_runtime::DOM_JS.len()
}

/// A ZDeceptron source line that carries a program.
///
/// Swift counted "lines of application Jif", and the repository's examples
/// are teaching files whose comments outnumber their code — `hello.zd` is
/// twelve lines of which six are prose. Counting those would flatter the
/// ratio by a factor of two, so they are excluded and the raw line count is
/// reported next to it.
pub fn code_lines(source: &str) -> usize {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .count()
}

/// One program, compiled, in Swift's units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emitted {
    pub name: String,
    /// Every line in the file, comments and blanks included.
    pub lines: usize,
    /// Lines that carry a program.
    pub code_lines: usize,
    pub client_js: usize,
    /// `client.js` plus the stylesheet, the entry document and the manifest.
    pub bundle: usize,
}

impl Emitted {
    /// Bytes of JavaScript per line of source, the runtime excluded.
    ///
    /// This is the marginal cost of a line: what the program adds to a
    /// bundle whose runtime is already there.
    pub fn bytes_per_line(&self) -> usize {
        self.client_js / self.code_lines.max(1)
    }

    /// The same, charging the whole shared runtime to this one program.
    ///
    /// This is the number for a single-page application that ships nothing
    /// else — the worst case, and the one that dominates at small sizes.
    pub fn bytes_per_line_with_runtime(&self) -> usize {
        (self.client_js + runtime_js_bytes()) / self.code_lines.max(1)
    }
}

/// Every `.zd` file in the repository worth sizing, in a fixed order.
fn survey_sources() -> Vec<(String, String)> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(repository_path("examples"))
        .expect("the examples directory exists")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "zd"))
        .collect();
    paths.sort();
    paths.push(repository_path("crates/zdc-bench/bench/row.zd"));

    // Named by repository-relative path, the same way `bundle_sizes` names
    // them. The name is not decoration: the emitter writes it into
    // `client.js`, so two spellings of the same file differ in byte count
    // by the difference in their lengths, and the two tables in this file
    // would not agree.
    let root = repository_path("");
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            (name, source)
        })
        .collect()
}

/// Every example that builds, sized; and every one that does not, with why.
///
/// Both halves are returned together because a bytes-per-line figure over a
/// subset is only honest if the subset is named.
pub fn survey() -> (Vec<Emitted>, Vec<(String, Vec<String>)>) {
    let mut built = Vec::new();
    let mut refused = Vec::new();
    for (name, source) in survey_sources() {
        match try_compile(&source, &name) {
            Ok(bundle) => built.push(Emitted {
                lines: source.lines().count(),
                code_lines: code_lines(&source),
                client_js: bundle.client_js.len(),
                bundle: bundle.client_js.len()
                    + bundle.styles_css.len()
                    + bundle.index_html.len()
                    + bundle.manifest_json.len(),
                name,
            }),
            Err(errors) => refused.push((name, errors)),
        }
    }
    (built, refused)
}

/// The empty-program baseline, in Swift's shape.
///
/// Swift's was six lines. This is six lines that do the least a ZDeceptron
/// program can do and still be one: hold a value and show it.
pub const NULL_PROGRAM: &str = "state greeting is client Text starting \"\"\n\
                                \n\
                                view\n\
                                \x20   Column\n\
                                \x20       Input greeting, hint is \"say something\"\n\
                                \x20       Text greeting\n";

/// The smallest program the compiler will build at all.
pub const SMALLEST_PROGRAM: &str = "view\n\x20   Text \"x\"\n";

/// Compile a source that is expected to build, or explain what refused it.
pub fn build(source: &str, name: &str) -> Emitted {
    let bundle = try_compile(source, name)
        .unwrap_or_else(|errors| panic!("{name} failed to compile:\n  {}", errors.join("\n  ")));
    Emitted {
        name: name.to_string(),
        lines: source.lines().count(),
        code_lines: code_lines(source),
        client_js: bundle.client_js.len(),
        bundle: bundle.client_js.len()
            + bundle.styles_css.len()
            + bundle.index_html.len()
            + bundle.manifest_json.len(),
    }
}

/// `n` client signals, each declared once and read once in the view.
///
/// The growth series' independent variable. Every line is a line a person
/// could have written, and nothing about the shape gets cheaper with `n`,
/// so a superlinear emission would show up here first.
pub fn program_with_signals(n: usize) -> String {
    let mut source = String::new();
    for i in 0..n {
        source.push_str(&format!("state s{i} is client Whole starting {i}\n"));
    }
    source.push_str("\nview\n    Column\n");
    for i in 0..n {
        source.push_str(&format!("        Text s{i}\n"));
    }
    source
}

/// A view nested `n` elements deep around a single leaf.
pub fn program_with_depth(n: usize) -> String {
    let mut source = String::from("state leaf is client Text starting \"leaf\"\n\nview\n");
    for i in 0..n {
        source.push_str(&format!("{}Column\n", " ".repeat(4 * (i + 1))));
    }
    source.push_str(&format!("{}Text leaf\n", " ".repeat(4 * (n + 1))));
    source
}

/// `defs` shared definitions reachable from each of `roots` roots.
///
/// §17.2 makes tier splitting reachability over the product of the
/// definition set and the root set. To measure the product rather than
/// either factor, the definitions must be *shared*: a chain of `defs`
/// functions, and `roots` server-placed signals each rooted at the head of
/// that chain. The source is O(defs + roots) lines and the reachable set is
/// `defs × roots` pairs, so any gap between the two is the pass's own.
///
/// Server placement is what mints a root here. `zdc build` refuses to emit
/// a server function (§16.5, M6), which is why this measures `split` and
/// `ifc` directly rather than going through the whole pipeline — those two
/// passes run on the program regardless of whether anything is emitted.
pub fn program_with_roots(defs: usize, roots: usize) -> String {
    let defs = defs.max(1);
    let mut source = String::from("function f0 with x\n    give x + 1\n");
    for i in 1..defs {
        source.push_str(&format!(
            "function f{i} with x\n    give f{} with x\n",
            i - 1
        ));
    }
    for i in 0..roots {
        source.push_str(&format!(
            "state v{i} is server Whole from f{} with {i}\n",
            defs - 1
        ));
    }
    source.push_str("\nview\n    Column\n");
    for i in 0..roots {
        source.push_str(&format!("        Text v{i}\n"));
    }
    source
}

/// What the two graph passes cost on one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphTimes {
    pub defs: usize,
    pub roots: usize,
    pub split: Duration,
    pub ifc: Duration,
}

impl GraphTimes {
    /// The size of the set §17.2 says the splitter walks.
    pub fn pairs(&self) -> usize {
        self.defs * self.roots
    }
}

/// Time `split` and `ifc` separately, averaged over `reps` runs.
///
/// Parsing and resolving are done once and outside the timed region: they
/// are linear in the source and not what is in question. The information-
/// flow pass is timed on its own because §17.3 is the pass most likely to
/// be blamed for a slow compiler and the only way to know is to separate it.
pub fn time_graph_passes(source: &str, reps: u32) -> GraphTimes {
    let reps = reps.max(1);
    let program = zdc_parser::parse(source).unwrap_or_else(|e| panic!("{}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| panic!("{}", errors[0].message));

    let started = Instant::now();
    for _ in 0..reps {
        std::hint::black_box(zdc_graph::split(&hir));
    }
    let split = started.elapsed() / reps;

    let tier_split = zdc_graph::split(&hir);
    let started = Instant::now();
    for _ in 0..reps {
        std::hint::black_box(zdc_graph::ifc(&hir, &tier_split));
    }
    let ifc = started.elapsed() / reps;

    GraphTimes {
        defs: hir.defs.len(),
        roots: tier_split.roots.len(),
        split,
        ifc,
    }
}

/// How many elements an index-recursive fold gets through before the host
/// refuses.
///
/// §17.4.10's finding, reduced to its cause. There are no local bindings in
/// ZDeceptron, so a fold cannot carry an accumulator through a loop;
/// §17.4.9's technique is index recursion, and stack depth is therefore
/// linear in the input. What is measured is the shape that emits — one
/// self-call per element — against the interpreter the rest of this suite
/// runs in. The number is that interpreter's recursion budget, not the
/// language's and not a browser's; what the language contributes is that
/// the depth grows with the input at all.
pub fn deepest_fold() -> usize {
    let folds = |n: usize| {
        let mut context = boa_engine::Context::default();
        let source = format!(
            "function sumFrom(xs, i) {{ if (i >= xs.length) return 0; \
             return xs[i] + sumFrom(xs, i + 1); }}\n\
             sumFrom(new Array({n}).fill(1), 0)"
        );
        context
            .eval(boa_engine::Source::from_bytes(source.as_bytes()))
            .is_ok()
    };

    // Bisection rather than a scan: the budget is a cliff, not a slope.
    let mut deepest = 1usize;
    let mut refused = 1usize << 14;
    assert!(folds(deepest), "a one-element fold must succeed");
    while deepest + 1 < refused {
        let middle = deepest + (refused - deepest) / 2;
        if folds(middle) {
            deepest = middle;
        } else {
            refused = middle;
        }
    }
    deepest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_blanks_are_not_program_lines() {
        assert_eq!(code_lines("# a\n\n  # b\nview\n    Text \"x\"\n"), 2);
    }

    #[test]
    fn the_null_program_is_six_lines_like_swifts() {
        assert_eq!(NULL_PROGRAM.lines().count(), SWIFT_NULL_PROGRAM_LINES);
    }

    #[test]
    fn the_generators_produce_the_sizes_they_claim() {
        assert_eq!(code_lines(&program_with_signals(8)), 8 + 8 + 2);
        let times = time_graph_passes(&program_with_roots(4, 4), 1);
        // Two singletons (§17.2.6) plus one endpoint per server signal.
        assert_eq!(times.roots, 4 + 2);
    }
}
