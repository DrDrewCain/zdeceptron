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

/// The runtime a program that renders a view and nothing else links.
///
/// `elements.js` is not in the sum: generated code never imports it
/// (§16.3.1), so it is not shipped. Uncompressed, and minified, because
/// since #135 that is what `zdc build` writes — the figure is measured
/// through the same call the command makes rather than projected from the
/// files on disk.
///
/// **This is a reference figure, not the gate's input.** The runtime is
/// several files and a bundle links a subset — `rpc.js` and `store.js`
/// only where the split found a crossing or a durable key, `foreign.js`
/// only where the program writes a `foreign … gives view`. Charging a
/// program a fixed sum is therefore wrong in both directions: it flatters
/// a live-sync program and penalises one with an FFI. Every gate below
/// uses [`Emitted::runtime_js`], which is measured from the bundle's own
/// import closure; this function names the floor that a plain rendering
/// program pays, which is what the `B/line+rt` column is relative to.
pub fn runtime_js_bytes() -> usize {
    // As a release build ships them: the `// $dev` assertions (#140) are
    // not downloaded by a reader, so charging them here would report a
    // cost nobody pays.
    let release = |source| zdc_runtime::for_mode(source, zdc_runtime::Mode::Release).len();
    release(zdc_runtime::SIGNAL_JS) + release(zdc_runtime::DOM_JS)
}

/// The runtime files one bundle actually links, in bytes.
///
/// The set comes from `Bundle::runtime`, which is the same closure the
/// emitter used to decide the import list — one decision rather than two
/// that have to agree. This is what makes "ships nothing it does not use"
/// a measured property here rather than an assumption baked into a sum.
pub fn linked_runtime_bytes(runtime: &std::collections::BTreeSet<&'static str>) -> usize {
    // Release, because the size claims in `BENCHMARKS.md` are claims about
    // what a reader downloads.
    linked_runtime_bytes_in(runtime, zdc_codegen::Mode::Release)
}

/// The same, for whichever build is asked about.
///
/// What a development build costs is worth measuring rather than
/// estimating: #140's whole argument is that the assertions are free to a
/// reader because they are stripped, and the number that makes that
/// checkable is the difference between the two.
pub fn linked_runtime_bytes_in(
    runtime: &std::collections::BTreeSet<&'static str>,
    mode: zdc_codegen::Mode,
) -> usize {
    zdc_codegen::runtime_files(runtime, mode)
        .iter()
        .map(|(_, source)| source.len())
        .sum()
}

/// The linked set minified but **not** stripped: what a release build
/// would weigh if the assertions shipped.
///
/// The third number the assertion survey needs since #135. Before the
/// minifier, "development minus release" was the cost of the `// $dev`
/// blocks and nothing else. Now two transformations separate those two
/// figures, and subtracting them would report the comments as assertions
/// — so the middle figure is measured instead of inferred, and the
/// assertions' cost is stated in the units that matter, which is bytes a
/// reader would have downloaded rather than bytes on disk.
pub fn linked_runtime_bytes_with_assertions(
    runtime: &std::collections::BTreeSet<&'static str>,
) -> usize {
    zdc_codegen::runtime_files(runtime, zdc_codegen::Mode::Development)
        .iter()
        .map(|(_, source)| zdc_codegen::minify::javascript(source).len())
        .sum()
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
    /// `client.js` as the emitter printed it.
    ///
    /// This, and not the minified length, is what the Swift comparison is
    /// made of: Swift's 800 bytes per line was its compiler's output, so
    /// measuring ours after a minifier the other side never had would be
    /// winning the comparison by changing it. Every per-line figure below
    /// is this number (#135).
    pub client_js: usize,
    /// `client.js` after minification — what a browser downloads.
    pub shipped_client_js: usize,
    /// `client.js` plus the stylesheet, the entry document and the manifest.
    pub bundle: usize,
    /// The runtime files this bundle links, in bytes — its own closure,
    /// not a constant, and minified as a release build ships it. See
    /// [`linked_runtime_bytes`].
    pub runtime_js: usize,
}

impl Emitted {
    /// Bytes of JavaScript per line of source, the runtime excluded.
    ///
    /// This is the marginal cost of a line: what the program adds to a
    /// bundle whose runtime is already there.
    pub fn bytes_per_line(&self) -> usize {
        self.client_js / self.code_lines.max(1)
    }

    /// The same, charging this program's own runtime closure to it.
    ///
    /// This is the number for a single-page application that ships nothing
    /// else — the worst case, and the one that dominates at small sizes.
    pub fn bytes_per_line_with_runtime(&self) -> usize {
        self.shipped() / self.code_lines.max(1)
    }

    /// Every byte of JavaScript a visitor downloads for this program.
    ///
    /// Both halves minified, because both halves are minified on the way
    /// out (#135). Adding an emitted `client.js` to a minified runtime
    /// would be a number that describes no build.
    pub fn shipped(&self) -> usize {
        self.shipped_client_js + self.runtime_js
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
                shipped_client_js: zdc_codegen::minify::javascript(&bundle.client_js).len(),
                bundle: bundle.client_js.len()
                    + bundle.styles_css.len()
                    + bundle.index_html.as_deref().map_or(0, str::len)
                    + bundle.manifest_json.len(),
                runtime_js: linked_runtime_bytes(&bundle.runtime),
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

/// The null program plus the one construct that links `foreign.js`.
///
/// Kept beside `NULL_PROGRAM` because the pair is the measurement: the
/// difference between what these two ship is exactly what a DOM-owning
/// foreign costs, and stating it as a diff is what stops the split from
/// being a way to make the headline number smaller than the truth.
pub const FOREIGN_VIEW_PROGRAM: &str = "foreign gauge is client\n\
                                        \x20   from  \"./gauge.js\" as \"mount\"\n\
                                        \x20   takes level is Whole\n\
                                        \x20   gives view\n\
                                        \n\
                                        state level is client Whole starting 40\n\
                                        \n\
                                        view\n\
                                        \x20   Column\n\
                                        \x20       gauge level is level\n";

/// Compile a source that is expected to build, or explain what refused it.
pub fn build(source: &str, name: &str) -> Emitted {
    let bundle = try_compile(source, name)
        .unwrap_or_else(|errors| panic!("{name} failed to compile:\n  {}", errors.join("\n  ")));
    Emitted {
        name: name.to_string(),
        lines: source.lines().count(),
        code_lines: code_lines(source),
        client_js: bundle.client_js.len(),
        shipped_client_js: zdc_codegen::minify::javascript(&bundle.client_js).len(),
        bundle: bundle.client_js.len()
            + bundle.styles_css.len()
            + bundle.index_html.as_deref().map_or(0, str::len)
            + bundle.manifest_json.len(),
        runtime_js: linked_runtime_bytes(&bundle.runtime),
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

/// `count` instantiations of a chain of components `depth` deep.
///
/// §16.10 states the components trade-off as a dilemma: *"either the
/// compiler inlines bodies into the parent's template, multiplying
/// template bytes and destroying per-component incremental compilation, or
/// a call site becomes a dynamic hole with its own clone, degrading toward
/// one clone per component."* Issue #209 asks which horn this compiler is
/// on and what it costs, and neither question has an answer without a
/// program whose component depth and count can be varied independently.
///
/// Each level declares one component whose body instantiates the next, so
/// the source is O(depth + count) lines while the fully expanded view is
/// depth × count bodies. A superlinear emission in either variable shows
/// up here and nowhere else in this file: `program_with_depth` nests
/// built-in elements, which the emitter has never had to expand.
///
/// `shared` is the other half of the question, and it is the half that
/// decides whether the bytes were avoidable. A component handed a literal
/// at each call site folds that literal into the markup, so no two copies
/// of its body are the same string and there is nothing a compiler could
/// have shared. A component reading one module-level signal has a hole
/// there instead, so every copy is byte-identical — and the bytes are then
/// a choice rather than a necessity.
pub fn program_with_components(depth: usize, count: usize, shared: bool) -> String {
    let depth = depth.max(1);
    let mut source = String::new();
    if shared {
        source.push_str("state caption is client Text starting \"caption\"\n\n");
    }
    source.push_str(
        "component C0 with label\n    \
         Column\n        \
         Heading label\n        \
         Text \"a static caption line\"\n\n",
    );
    for level in 1..depth {
        source.push_str(&format!(
            "component C{level} with label\n    \
             Column\n        \
             C{} label\n        \
             Text \"a static caption line\"\n\n",
            level - 1
        ));
    }
    source.push_str("view\n    Column\n");
    for i in 0..count {
        let argument = if shared {
            "caption".to_string()
        } else {
            format!("\"card {i}\"")
        };
        source.push_str(&format!("        C{} {argument}\n", depth - 1));
    }
    source
}

/// The same view with the components written out by hand.
///
/// The control the component measurement needs: whatever the emitter does
/// with a component, this is what the programmer would otherwise have
/// typed, so the difference between the two is what components cost. It is
/// the *source* that differs, not the tree — both render the same page.
pub fn program_without_components(depth: usize, count: usize, shared: bool) -> String {
    let depth = depth.max(1);
    let mut source = String::new();
    if shared {
        source.push_str("state caption is client Text starting \"caption\"\n\n");
    }
    source.push_str("view\n    Column\n");
    for i in 0..count {
        let argument = if shared {
            "caption".to_string()
        } else {
            format!("\"card {i}\"")
        };
        write_inlined(depth, 8, &argument, &mut source);
    }
    source
}

/// One instantiation of the component chain, written out.
///
/// `C{n}`'s body is a `Column` holding `C{n-1}` and a caption, and `C0`'s
/// is a `Column` holding a heading and a caption, so the expansion is a
/// nest of `Column`s each with the caption after the one inside it. Written
/// recursively because that is the shape; a loop got the caption order
/// wrong at depth 2 and the compiler caught it.
fn write_inlined(remaining: usize, indent: usize, argument: &str, out: &mut String) {
    let pad = " ".repeat(indent);
    let inner = " ".repeat(indent + 4);
    out.push_str(&format!("{pad}Column\n"));
    if remaining == 1 {
        out.push_str(&format!("{inner}Heading {argument}\n"));
    } else {
        write_inlined(remaining - 1, indent + 4, argument, out);
    }
    out.push_str(&format!("{inner}Text \"a static caption line\"\n"));
}

/// The bytes a module spends on static markup.
///
/// Every `template('…')` argument in an emission, summed. This is the
/// quantity §16.10's dilemma is about: the byte count that inlining
/// multiplies, as distinct from the module's total size, which also
/// carries the walk to each hole and the bindings attached there.
pub fn template_bytes(client_js: &str) -> usize {
    let mut total = 0;
    let mut rest = client_js;
    while let Some(open) = rest.find("template('") {
        rest = &rest[open + "template('".len()..];
        // The emitter escapes every quote it interpolates (§16.3.5), so
        // the first unescaped `'` ends the literal.
        let mut end = 0;
        let bytes = rest.as_bytes();
        while end < bytes.len() && !(bytes[end] == b'\'' && (end == 0 || bytes[end - 1] != b'\\')) {
            end += 1;
        }
        total += end;
        rest = &rest[end.min(bytes.len())..];
    }
    total
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
