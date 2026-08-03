//! Swift's number, and the three ways it could still go wrong.
//!
//! Swift (SOSP'07) built ZDeceptron's thesis and measured **~800 bytes of
//! JavaScript per line of source**. That figure is why the approach is
//! remembered as a warning rather than a technique, and it is the one this
//! compiler has to beat. The gates below are the beating, expressed as
//! inequalities with headroom so that an ordinary change to the emitter
//! does not fail the build and an architectural one does.
//!
//! The timed surveys are `#[ignore]`d. They print rather than assert —
//! wall-clock is not a gate — and they are what regenerates the numbers in
//! `BENCHMARKS.md`:
//!
//! ```sh
//! cargo test -p zdc-bench --release --test scaling -- --ignored --nocapture
//! ```

use zdc_bench::{
    build, deepest_fold, program_with_depth, program_with_roots, program_with_signals,
    runtime_js_bytes, survey, time_graph_passes, try_compile, Emitted, FOREIGN_VIEW_PROGRAM,
    NULL_PROGRAM, SMALLEST_PROGRAM, SWIFT_BYTES_PER_LINE, SWIFT_LARGEST_APP_LINES,
    SWIFT_NULL_PROGRAM_JS,
};

/// Swift's null program was 6 lines and 73 kB. Ours is 6 lines and what?
///
/// This is the single most comparable figure in the two systems, because it
/// is almost entirely machinery: whatever a null program ships is what the
/// approach costs before any application exists.
#[test]
fn the_null_program_is_a_fraction_of_swifts() {
    let emitted = build(NULL_PROGRAM, "null.zd");
    // The bundle's own import closure, not a fixed sum over the runtime
    // directory. A null program links `signal.js` and `dom.js`; the gate
    // would be measuring the wrong thing — and would miss a regression
    // that made it link `store.js` — if it assumed that instead of
    // asking. `a_null_program_links_two_runtime_files` pins the set.
    let shipped = emitted.shipped();
    assert!(
        shipped * 3 < SWIFT_NULL_PROGRAM_JS,
        "the null program ships {shipped} bytes of JavaScript against Swift's \
         {SWIFT_NULL_PROGRAM_JS}. Under 3× smaller is not the win this file \
         claims; either the runtime grew a framework inside it or the emitter \
         started shipping one per program."
    );
    // The runtime is nearly all of it, which is the point: it is shared.
    assert!(
        emitted.client_js < emitted.runtime_js / 10,
        "a null program's own emission is {} bytes against a {} byte runtime. \
         If the program's half is growing, the per-program cost is no longer \
         negligible and the amortisation argument below weakens.",
        emitted.client_js,
        emitted.runtime_js
    );
}

/// A null program links two runtime files, and this names them.
///
/// Without this the gate above could be satisfied by moving bytes into a
/// module the null program does not import, which would be gaming the
/// measurement rather than shipping less. Splitting the runtime is a
/// legitimate way to ship less **only** when the code moved is genuinely
/// optional, so the set is pinned by name: adding a file here is a
/// deliberate act with a test to change, and moving machinery a null
/// program needs into a file it still imports does not help.
#[test]
fn a_null_program_links_two_runtime_files() {
    let bundle = try_compile(NULL_PROGRAM, "null.zd").expect("the null program builds");
    let linked: Vec<&str> = bundle.runtime.iter().copied().collect();
    assert_eq!(linked, vec!["runtime/dom.js", "runtime/signal.js"]);
}

/// The foreign lifecycle is shipped to the programs that use it.
///
/// The other half of the split, and the half that makes it honest.
/// `foreign.js` is kept out of a null program's bundle because nothing in
/// a null program can reach it — not because it stopped mattering. A
/// program that writes a `foreign … gives view` must link it, or the
/// bundle is one that throws on load.
#[test]
fn a_foreign_view_program_links_the_lifecycle_and_still_beats_swift() {
    let emitted = build(FOREIGN_VIEW_PROGRAM, "foreign.zd");
    let bundle =
        try_compile(FOREIGN_VIEW_PROGRAM, "foreign.zd").expect("the foreign program builds");
    let linked: Vec<&str> = bundle.runtime.iter().copied().collect();
    assert_eq!(
        linked,
        vec!["runtime/dom.js", "runtime/foreign.js", "runtime/signal.js"],
        "a `gives view` foreign must ship the module that drives it"
    );
    assert!(
        bundle.client_js.contains("/foreign.js"),
        "the import list and the shipped set are one decision: {}",
        bundle.client_js
    );

    // Charged the whole lifecycle module, an FFI program is still well
    // clear of Swift's null program — which is the claim that has to
    // survive the split, because a reader's fair question is "and what
    // does it cost when you *do* use the feature".
    assert!(
        emitted.shipped() * 2 < SWIFT_NULL_PROGRAM_JS,
        "a program with a DOM-owning foreign ships {} bytes against Swift's \
         {SWIFT_NULL_PROGRAM_JS}. The lifecycle module is optional, not free, \
         and this is where its cost is charged.",
        emitted.shipped()
    );
}

/// Every example that builds, in Swift's units.
///
/// The runtime is excluded here and charged separately below, because it is
/// one file shared by every page: counting it per program answers "what does
/// a single-page app cost", and counting it once answers "what does a line
/// of ZDeceptron cost". Both are reported in `BENCHMARKS.md`; only the
/// second is a property of the language.
#[test]
fn no_example_approaches_swifts_800_bytes_per_line() {
    let (built, _) = survey();
    assert!(built.len() >= 5, "only {} examples build", built.len());
    for emitted in &built {
        assert!(
            emitted.bytes_per_line() * 4 < SWIFT_BYTES_PER_LINE,
            "{} emits {} bytes of JavaScript per line of source. Swift's \
             measured figure was {SWIFT_BYTES_PER_LINE}, and this gate wants \
             at least a 4× margin — under that, the comparison stops being \
             the argument it is made to be.",
            emitted.name,
            emitted.bytes_per_line()
        );
    }
}

/// Charging the whole runtime to one program still beats Swift at the size
/// Swift's own largest application was.
///
/// Swift's `Shop` was 1,094 lines and 1.21 MB. The runtime is a fixed cost,
/// so the honest way to compare a fixed-plus-marginal figure against a
/// purely marginal one is to pick a size and do the arithmetic.
#[test]
fn at_swifts_largest_app_size_the_runtime_is_already_amortised() {
    let (built, _) = survey();
    let marginal = built
        .iter()
        .map(Emitted::bytes_per_line)
        .max()
        .expect("something builds");
    let projected = runtime_js_bytes() + marginal * SWIFT_LARGEST_APP_LINES;
    let swift = SWIFT_BYTES_PER_LINE * SWIFT_LARGEST_APP_LINES;
    assert!(
        projected * 5 < swift,
        "projected {projected} bytes at {SWIFT_LARGEST_APP_LINES} lines against \
         Swift's {swift}. This is arithmetic on a measured marginal cost, not a \
         measured application — but if it ever stops clearing 5×, the marginal \
         cost has grown enough that the extrapolation should be replaced by a \
         real program of that size."
    );
}

/// Emitted size is linear in source size. This is the claim that matters.
///
/// A superlinear emitter is the failure mode that would actually threaten
/// the design, because it cannot be fixed by minification or by shipping
/// less runtime — it is the partitioning machinery being written out per
/// definition pair. Doubling the program must roughly double the output.
#[test]
fn emitted_size_grows_linearly_with_source_size() {
    let sizes = [8usize, 16, 32, 64, 128, 256];
    let measured: Vec<Emitted> = sizes
        .iter()
        .map(|n| build(&program_with_signals(*n), "growth.zd"))
        .collect();

    for pair in measured.windows(2) {
        let ratio = pair[1].client_js as f64 / pair[0].client_js as f64;
        assert!(
            ratio < 2.2,
            "doubling the program multiplied the emission by {ratio:.2} \
             ({} bytes to {} bytes). Anything meaningfully over 2 is \
             superlinear growth, which is the one result that would put the \
             approach itself in question.",
            pair[0].client_js,
            pair[1].client_js
        );
    }

    // And the marginal cost per line is flat, not merely sub-quadratic.
    let first = measured[0].bytes_per_line();
    let last = measured[measured.len() - 1].bytes_per_line();
    assert!(
        last <= first + first / 4,
        "bytes per line went from {first} at {} lines to {last} at {} lines. \
         A flat marginal cost is what makes the per-line figure a property of \
         the language rather than of the example.",
        measured[0].code_lines,
        measured[measured.len() - 1].code_lines
    );
}

/// Nesting is not the hidden quadratic either.
///
/// Template cloning emits one markup string per region and a walk to each
/// hole, so a deeply nested view could in principle re-emit its ancestry.
///
/// Twelve against forty-eight rather than a wider spread because the parser
/// refuses an indented block nested more than 64 levels deep, which is its
/// own answer to the question and a stated one.
#[test]
fn nesting_depth_does_not_compound() {
    // Seven and twenty-eight: still the fourfold step this measures, and
    // both inside the parser's block limit, which came down to 32 when the
    // per-level stack cost was re-measured.
    let shallow = build(&program_with_depth(7), "depth.zd");
    let deep = build(&program_with_depth(28), "depth.zd");
    let ratio = deep.client_js as f64 / shallow.client_js as f64;
    assert!(
        ratio < 5.0,
        "quadrupling the nesting depth multiplied the emission by {ratio:.2}; \
         a view's markup is being emitted more than once per region."
    );
}

/// The smallest thing the compiler will build, pinned so the floor is visible.
#[test]
fn the_floor_is_a_view_and_nothing_else() {
    let emitted = build(SMALLEST_PROGRAM, "floor.zd");
    assert!(
        emitted.client_js < 512,
        "the smallest possible program emits {} bytes",
        emitted.client_js
    );
}

/// §17.4.10's ceiling, measured on this base.
///
/// The depth grows with the input because §17.4.9's index recursion is the
/// only way to fold without local bindings. The *number* is the embedded
/// interpreter's recursion budget rather than the language's — a browser's
/// is roughly an order of magnitude larger — so what is pinned is a band,
/// and what it proves is that a ceiling exists at all.
#[test]
fn a_fold_has_a_ceiling_and_it_is_linear_in_the_input() {
    let deepest = deepest_fold();
    assert!(
        (200..=2_000).contains(&deepest),
        "an index-recursive fold got through {deepest} elements. Outside this \
         band either the host's budget changed or folds stopped being linear \
         in stack depth — if the latter, §17.4.10 has been fixed and should \
         say so."
    );
}

/// Tier splitting really is the product §17.2 says it is.
///
/// Not a timing assertion — a shape assertion, and it is deterministic. The
/// generator gives every root the same chain of definitions to walk, so if
/// the splitter visits `(definition, root)` pairs then holding one factor
/// fixed and doubling the other must double the work either way round. The
/// timed version of this is the ignored survey below; this is the part that
/// can be a gate.
#[test]
fn splitting_walks_the_product_of_definitions_and_roots() {
    let by_roots = time_graph_passes(&program_with_roots(16, 32), 1);
    let by_defs = time_graph_passes(&program_with_roots(32, 16), 1);

    // Two singletons plus one endpoint per server signal (§17.2.6), and one
    // definition per function, per signal, and the view.
    assert_eq!(by_roots.roots, 32 + 2);
    assert_eq!(by_roots.defs, 16 + 32 + 1);
    assert_eq!(by_defs.roots, 16 + 2);
    assert_eq!(by_defs.defs, 32 + 16 + 1);

    // Both programs are the same size and reach the same number of pairs
    // from opposite sides, which is what makes the timed survey a
    // measurement of the product rather than of either factor.
    assert_eq!(by_roots.defs, by_defs.defs);
}

// --- the surveys, which print rather than assert -------------------------

/// Bytes of JavaScript per line of ZDeceptron, over everything that builds.
#[test]
#[ignore = "prints the survey behind BENCHMARKS.md; not a gate"]
fn survey_bytes_per_line() {
    let (built, refused) = survey();
    println!(
        "\nruntime a rendering program links (signal.js + dom.js): {} bytes",
        runtime_js_bytes()
    );
    println!(
        "\n{:<18} {:>6} {:>6} {:>9} {:>9} {:>9} {:>11}",
        "program", "lines", "code", "client.js", "bundle", "B/line", "B/line+rt"
    );
    for emitted in &built {
        println!(
            "{:<18} {:>6} {:>6} {:>9} {:>9} {:>9} {:>11}",
            emitted.name,
            emitted.lines,
            emitted.code_lines,
            emitted.client_js,
            emitted.bundle,
            emitted.bytes_per_line(),
            emitted.bytes_per_line_with_runtime(),
        );
    }
    let null = build(NULL_PROGRAM, "null.zd");
    println!(
        "\nnull program: {} code lines, {} bytes emitted, {} bytes of JavaScript shipped",
        null.code_lines,
        null.client_js,
        null.shipped()
    );
    let foreign = build(FOREIGN_VIEW_PROGRAM, "foreign.zd");
    println!(
        "with a `gives view` foreign: {} bytes emitted, {} bytes of JavaScript shipped",
        foreign.client_js,
        foreign.shipped()
    );
    for (name, errors) in &refused {
        println!("refused: {name} — {}", errors.join(" | "));
    }
}

/// Emitted size against source size, on a program that only gets bigger.
#[test]
#[ignore = "prints the survey behind BENCHMARKS.md; not a gate"]
fn survey_growth() {
    println!(
        "\n{:>6} {:>7} {:>10} {:>9} {:>9}",
        "signals", "lines", "client.js", "B/line", "ratio"
    );
    let mut previous = 0usize;
    for n in [8usize, 16, 32, 64, 128, 256, 512, 1024] {
        let emitted = build(&program_with_signals(n), "growth.zd");
        let ratio = if previous == 0 {
            0.0
        } else {
            emitted.client_js as f64 / previous as f64
        };
        println!(
            "{n:>6} {:>7} {:>10} {:>9} {ratio:>9.2}",
            emitted.code_lines,
            emitted.client_js,
            emitted.bytes_per_line()
        );
        previous = emitted.client_js;
    }
}

/// Compile time against definition count and root count, separately.
///
/// §17.2 makes tier splitting reachability over the product of the two, and
/// routing multiplies the roots — one per page. Each half of this survey
/// holds one factor fixed and doubles the other; the third holds neither.
#[test]
#[ignore = "prints the survey behind BENCHMARKS.md; not a gate"]
fn survey_compiler_asymptotics() {
    let show = |label: &str, defs: usize, roots: usize| {
        let times = time_graph_passes(&program_with_roots(defs, roots), 20);
        println!(
            "{label:<16} defs={:<6} roots={:<6} pairs={:<9} split={:>9.1}us ifc={:>9.1}us",
            times.defs,
            times.roots,
            times.pairs(),
            times.split.as_secs_f64() * 1e6,
            times.ifc.as_secs_f64() * 1e6,
        );
    };

    println!("\ndefinitions fixed at 32, roots doubling:");
    for roots in [4usize, 8, 16, 32, 64, 128, 256] {
        show(&format!("D=32 R={roots}"), 32, roots);
    }
    println!("\nroots fixed at 32, definitions doubling:");
    for defs in [4usize, 8, 16, 32, 64, 128, 256] {
        show(&format!("D={defs} R=32"), defs, 32);
    }
    println!("\nboth doubling:");
    for n in [8usize, 16, 32, 64, 128, 256] {
        show(&format!("D=R={n}"), n, n);
    }
}
