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
    build, deepest_fold, endpoint_duplication, linked_runtime_bytes_in,
    linked_runtime_bytes_with_assertions, program_with_components, program_with_depth,
    program_with_roots, program_with_shared_endpoints, program_with_signals,
    program_without_components, repository_path, runtime_js_bytes, survey, template_bytes,
    time_graph_passes, try_compile, Emitted, FOREIGN_VIEW_PROGRAM, NULL_PROGRAM, SMALLEST_PROGRAM,
    SWIFT_BYTES_PER_LINE, SWIFT_LARGEST_APP_LINES, SWIFT_NULL_PROGRAM_JS,
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
    //
    // Both sides are the *shipped* lengths (#135). Mixing them would make
    // this ratio meaningless, and it moved for a real reason when the
    // minifier landed: the runtime is prose-heavy and lost about 70% of
    // itself, while `client.js` is generated and had almost no comments to
    // lose. So the program's share of a bundle genuinely rose — from
    // roughly a thirtieth to roughly a seventh — and the threshold is
    // restated at what is now measured rather than left at a number that
    // used to have room. Fivefold, on a measurement that clears it by
    // nearly three times again.
    assert!(
        emitted.shipped_client_js * 5 < emitted.runtime_js,
        "a null program ships {} bytes of its own against a {} byte runtime. \
         If the program's half is growing, the per-program cost is no longer \
         negligible and the amortisation argument below weakens.",
        emitted.shipped_client_js,
        emitted.runtime_js
    );
}

/// How close to the wall the gate above is allowed to get.
///
/// Two kilobytes: enough that a normal change has room, small enough that
/// it is reached long before the claim is.
const HEADROOM_FLOOR: usize = 2_048;

/// **The gate above must fail loudly rather than silently run out.**
///
/// `shipped * 3 < 73_000` is an inequality, and an inequality says nothing
/// until the moment it says everything. That is not hypothetical here: the
/// margin had drifted to **five bytes** while the prose beside it still
/// claimed the ceilings sat "roughly 50% above what is emitted today" —
/// they were at 99.98%. Nothing failed, because nothing was watching the
/// distance. The next person to add six bytes to `dom.js` would have been
/// told their unrelated change broke a Swift comparison.
///
/// So the distance is measured too. Crossing this floor is not a bug and
/// the message says so — it is the point at which the answer stops being
/// "carry on" and becomes a decision: shrink the runtime, or move the code
/// into a module only the programs that need it link, which is what
/// `list.js`, `foreign.js` and `markup.js` already are.
#[test]
fn the_size_gate_keeps_room_to_warn_before_it_fails() {
    let shipped = build(NULL_PROGRAM, "null.zd").shipped();
    let ceiling = SWIFT_NULL_PROGRAM_JS / 3;
    let headroom = ceiling.saturating_sub(shipped);
    assert!(
        headroom >= HEADROOM_FLOOR,
        "the null program ships {shipped} bytes against a {ceiling} byte \
         ceiling, leaving {headroom} — under the {HEADROOM_FLOOR} this test \
         keeps in reserve.\n\nThis is not a failure of whatever change you \
         just made; it is the runtime having grown to where the next change \
         cannot fit. The fix is not to raise the ceiling, which is a claim \
         about Swift and not ours to move. Either shrink the runtime, or \
         move what you added into a module linked only by the programs that \
         use it — `list.js`, `foreign.js` and `markup.js` are each that, and \
         `a_null_program_links_two_runtime_files` is what stops the split \
         from becoming a way of hiding bytes."
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

    // What each of the two transformations costs, per linked set. A
    // reader downloads the `shipped` column; only `zdc dev` serves
    // `source`. The middle column is measured rather than inferred (#135):
    // two things now separate the ends — the `// $dev` assertions and the
    // comments — and subtracting the ends would report one as the other.
    println!(
        "\n{:<22} {:>8} {:>10} {:>12} {:>11}",
        "linked set", "shipped", "+assertions", "source", "assertions"
    );
    // `list.js` is in both sets deliberately. The reconciler assertion is
    // the larger of the two, and #207 moved it out of `dom.js` into a
    // module a bundle links only for a program with an `each` — so a set
    // without `list.js` measures the mechanism as costing nothing, which
    // is true of that set and false of the claim.
    for (label, modules) in [
        (
            "signal + dom + list",
            &["runtime/dom.js", "runtime/list.js", "runtime/signal.js"][..],
        ),
        (
            "+ wire, rpc, store",
            &[
                "runtime/dom.js",
                "runtime/list.js",
                "runtime/rpc.js",
                "runtime/signal.js",
                "runtime/store.js",
                "runtime/wire.js",
            ][..],
        ),
    ] {
        let set: std::collections::BTreeSet<&'static str> = modules.iter().copied().collect();
        let shipped = linked_runtime_bytes_in(&set, zdc_codegen::Mode::Release);
        let with_assertions = linked_runtime_bytes_with_assertions(&set);
        let source = linked_runtime_bytes_in(&set, zdc_codegen::Mode::Development);
        println!(
            "{label:<22} {shipped:>8} {with_assertions:>10} {source:>12} {:>11}",
            with_assertions - shipped
        );
    }
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

/// Compile time against the size of the *view*, which is the axis the two
/// surveys above hold fixed.
///
/// `split` and `ifc` are functions of the definition set and the root set,
/// and `program_with_roots` varies exactly those. Neither of them says
/// anything about a program that is one root and a great deal of markup —
/// which is the shape a person actually types, and the shape the editor
/// re-analyses on every keystroke. This survey varies the view and leaves
/// everything else alone.
///
/// It is the survey that found issue #8's quadratic, and it is here rather
/// than in a comment because the emitter's cost is invisible to every other
/// measurement in this file: the walk it schedules comes out the same
/// however long the scheduling takes, so no byte count could see it.
#[test]
#[ignore = "prints the survey behind BENCHMARKS.md; not a gate"]
fn survey_emitter_growth() {
    println!(
        "\n{:>8} {:>8} {:>11} {:>12} {:>10}",
        "signals", "nodes", "client.js", "emit", "us/node"
    );
    for n in [8usize, 16, 32, 64, 128, 256, 512, 1024] {
        let measured = zdc_bench::measure_emission(&program_with_signals(n), "growth.zd", 5);
        println!(
            "{n:>8} {:>8} {:>11} {:>10.3}ms {:>10.2}",
            measured.nodes,
            measured.client_js,
            measured.emit.as_secs_f64() * 1e3,
            measured.per_node(),
        );
    }
}

// --- components (spec §16.10, issue #209) --------------------------------
//
// §16.10 states the components trade-off as a dilemma and does not say
// which side this compiler is on:
//
// > "A component's shape is known only after inlining, so either the
// > compiler inlines bodies into the parent's template, multiplying
// > template bytes and destroying per-component incremental compilation,
// > or a call site becomes a dynamic hole with its own clone, degrading
// > toward one clone per component."
//
// It is the first: instantiation copies the body into the parent's
// template, so the whole view is still one `template()` and one clone.
// What that costs was never measured, and three separate things scale with
// component use, so "multiplying" could have meant almost anything. The
// gates below pin what it does mean — bytes linear in depth × count, and
// nothing at all over the same tree written by hand — and, in the last
// one, what the copying still wastes.

/// One instantiation's worth of static markup, at a given chain depth.
fn template_bytes_per_instantiation(depth: usize, count: usize, shared: bool) -> usize {
    let source = program_with_components(depth, count, shared);
    let bundle = try_compile(&source, "components.zd").expect("a component program builds");
    template_bytes(&bundle.client_js) / count
}

/// A component costs nothing over writing its body out at each call site.
///
/// This is the first half of the answer to §16.10, and it is the half a
/// reader is likely to assume rather than check: since instantiation is a
/// copy, the emission for `k` instantiations is the emission for the same
/// tree typed `k` times, to the byte. There is no per-component wrapper,
/// no extra clone, and no anchor pair — a call site is not a hole.
///
/// The four-byte allowance is the source path the emitter writes into the
/// module header (`components.zd` against `inline.zd`), which is the only
/// thing that differs between the two programs.
#[test]
fn a_component_emits_what_writing_it_out_would_have() {
    for shared in [false, true] {
        for depth in [1usize, 2, 4] {
            for count in [1usize, 5, 20] {
                let with = program_with_components(depth, count, shared);
                let without = program_without_components(depth, count, shared);
                let with = try_compile(&with, "components.zd").expect("builds");
                let without = try_compile(&without, "inline.zd").expect("builds");
                assert_eq!(
                    template_bytes(&with.client_js),
                    template_bytes(&without.client_js),
                    "depth {depth}, count {count}, shared {shared}: a component's markup must \
                     be the markup its body would have produced inline"
                );
                let difference = with.client_js.len().abs_diff(without.client_js.len());
                assert!(
                    difference <= 4,
                    "depth {depth}, count {count}, shared {shared}: {} bytes with components \
                     against {} without. The two programs describe the same tree, so anything \
                     beyond the length of the source path in the header is a per-component \
                     cost §16.10 does not account for.",
                    with.client_js.len(),
                    without.client_js.len()
                );
            }
        }
    }
}

/// "Multiplying template bytes" is linear in depth × count, not worse.
///
/// The number that matters is the *marginal* one: what one more
/// instantiation adds. If that were growing with the count, component use
/// would be superlinear and §16.10's word "multiplying" would be the right
/// one; measured, it is flat — 70 bytes at depth 1, 130 at depth 2, 250 at
/// depth 4, whatever the count. Flat in count and linear in depth is the
/// best an inlining strategy can do, and it is what this one does.
#[test]
fn component_bytes_grow_linearly_in_depth_and_count() {
    for depth in [1usize, 2, 4] {
        let one = template_bytes_per_instantiation(depth, 1, true);
        let twenty = template_bytes_per_instantiation(depth, 20, true);
        assert!(
            twenty <= one,
            "at depth {depth} an instantiation costs {one} bytes of markup on its own and \
             {twenty} bytes each when there are twenty of them. A marginal cost that rises \
             with the count is a superlinear emission, which is the reading of §16.10 this \
             measurement exists to rule out."
        );
    }
    // Linear in depth: each further level of nesting adds the same 60
    // bytes to every instantiation. Stated as a bound with headroom, since
    // the constant is a property of the component body this file builds.
    let marginal: Vec<usize> = [1usize, 2, 4]
        .into_iter()
        .map(|depth| template_bytes_per_instantiation(depth, 20, true))
        .collect();
    assert!(
        marginal[2] <= marginal[0] + 4 * (marginal[1] - marginal[0]),
        "an instantiation of a chain 1, 2 and 4 components deep costs {marginal:?} bytes of \
         markup. Depth is a chain, so the cost of depth 4 should be about the cost of depth 1 \
         plus three times what one further level adds; a number well above that means \
         instantiation is compounding rather than nesting."
    );
}

/// What the inlining strategy still wastes, quantified rather than argued.
///
/// When a component's arguments are holes rather than literals — one
/// module-level signal read by every instantiation — every copy of its body
/// is the *same string*, and the emitter writes all twenty. At depth 4 and
/// twenty instantiations that is 4,750 of 5,026 bytes of markup that a
/// shared-template emission would not have needed.
///
/// This is the residue of the §16.10 decision, and it is gated so that it
/// cannot be quietly fixed without the number in `BENCHMARKS.md` moving,
/// or quietly grow without the build failing. It is not a defect: sharing
/// the string would trade these bytes for either a second clone per
/// instantiation or a concatenation at module load, and neither has been
/// measured. What is settled is the size of what is on the table.
#[test]
fn identical_component_bodies_are_each_written_out_in_full() {
    let source = program_with_components(4, 20, true);
    let bundle = try_compile(&source, "components.zd").expect("builds");
    let markup = template_bytes(&bundle.client_js);
    let one = template_bytes_per_instantiation(4, 1, true);
    let duplicated = markup - one;
    assert_eq!(markup, 5_026, "the markup twenty instantiations emit");
    assert_eq!(one, 276, "the markup one instantiation emits");
    assert_eq!(
        duplicated, 4_750,
        "bytes of markup that are a copy of a body already in the template"
    );
    assert!(
        duplicated * 10 >= markup * 9,
        "{duplicated} of {markup} bytes are copies. BENCHMARKS.md reports this as 95% of the \
         markup; below 90% the sentence is wrong."
    );
}

// --- one bundle per root (spec §17.7, §16.3.12 rule 3, issue #23) ---------
//
// §17.7 emits one function bundle per server root, and §16.3.12 invariant 4
// forbids a bundle from importing a generated module — a shared generated
// module is exactly the import edge that would make the tier split
// analysable only by reading `import` statements, so "no cross-module
// imports" is the *syntactic evidence* for the invariant rather than an
// accident of the emitter. The price is that anything two roots both reach
// is written into both files.
//
// Issue #23 states the price and says no number exists for it: *"a server
// signal read by five endpoint derivations is recomputed inside all five
// bundles. A helper reachable from twenty endpoints is emitted twenty
// times."* These gates are the number. They measure; they do not fix, and
// the trade is not one an optimisation could make on its own — invariant 4
// is an information-flow property and the bytes are what it costs.

/// The mechanism, before the bytes: every bundle carries the whole of what
/// it reaches, and recomputes the shared signal itself.
///
/// This is the claim #23 makes, asserted rather than described. Five
/// endpoints read one server signal `shared`; all five files compute it, and
/// all five carry the helper that computes it. Nothing in the five imports
/// anything, which is the invariant the duplication is buying and is checked
/// here so that a later change cannot buy back the bytes by quietly
/// forfeiting it.
#[test]
fn every_endpoint_carries_and_recomputes_what_it_shares() {
    let source = program_with_shared_endpoints(1, 5, false);
    let bundle = try_compile(&source, "shared.zd").expect("a shared-endpoint program builds");
    assert_eq!(bundle.functions.len(), 5, "one file per endpoint");

    for function in &bundle.functions {
        assert!(
            function.source.contains("function w0("),
            "{} does not carry the helper it reaches:\n{}",
            function.path,
            function.source
        );
        assert!(
            function.source.contains("const shared = "),
            "{} does not recompute the shared signal:\n{}",
            function.path,
            function.source
        );
        // §16.3.12 assertion A and invariant 4, as a property of the bytes.
        // If this ever fails, the duplication below has been traded for the
        // import edge rather than for anything else, and that is a decision
        // about the invariant and not a size win.
        assert!(
            !function.source.contains("import "),
            "{} imports a module; invariant 4 is what the duplication buys:\n{}",
            function.path,
            function.source
        );
    }
}

/// **The growth curve.** Duplication is linear in the number of endpoints,
/// exactly: each further endpoint costs one more whole copy.
///
/// One size cannot tell a constant from a slope, so this asserts the law at
/// five sizes spanning 20×, the way `a reorder's move count against the
/// list's length` does. The law is `dup(N) = (N - 1) x dup(2)` and it holds
/// to the byte, which is what makes "emitted twenty times" a description of
/// the emitter rather than a figure of speech.
///
/// Both arms are checked because they bound the answer from either side: the
/// arithmetic arm shares only an author's helper, the collection arm shares
/// the emitter's own intrinsics as well, and the slope is the size of what
/// is shared in both.
#[test]
fn cross_root_duplication_is_one_further_copy_per_endpoint() {
    for collections in [false, true] {
        let of = |endpoints| {
            endpoint_duplication(
                &program_with_shared_endpoints(1, endpoints, collections),
                "shared.zd",
            )
        };
        let one = of(1);
        assert_eq!(
            one.duplicated, 0,
            "a program with a single endpoint duplicates nothing; there is nothing to duplicate \
             against"
        );

        let slope = of(2).duplicated;
        assert!(slope > 0, "two endpoints must share something");
        for endpoints in [5usize, 10, 20, 40] {
            let measured = of(endpoints);
            assert_eq!(
                measured.duplicated,
                (endpoints - 1) * slope,
                "{endpoints} endpoints (collections {collections}) duplicated {} bytes against \
                 the {} that one more whole copy each predicts. Any gap means the emitter has \
                 started sharing — or specialising — something across roots, and BENCHMARKS.md's \
                 growth curve is describing an emitter that no longer exists.",
                measured.duplicated,
                (endpoints - 1) * slope
            );
        }
    }
}

/// What is duplicated is mostly **the emitter's own machinery**, not the
/// program's.
///
/// The figures `BENCHMARKS.md` quotes, pinned so that neither the prose nor
/// the emitter can move without the other. An endpoint that touches a
/// `durable` list keyed by a `durable` map — `examples/voting-board.zd`'s
/// shape — is 2,282 bytes, and 1,569 of them are the collection intrinsics
/// the emitter inlines because invariant 4 leaves it nowhere else to put
/// them. Forty bytes of the file are about this endpoint.
///
/// This is the server-side twin of #136's finding that a routed site's real
/// per-page duplication was `base.css` rather than the modules: in both
/// halves of the compiler the bytes written twice are the shared machinery,
/// and in both halves the author's own code is the small part.
#[test]
fn a_collection_endpoint_duplicates_the_emitters_intrinsics() {
    let source = program_with_shared_endpoints(1, 2, true);
    let bundle = try_compile(&source, "shared.zd").expect("builds");
    let first = &bundle.functions[0].source;

    // The intrinsics run, from the first one the emitter writes to the
    // first line of the program's own code after them.
    let start = first
        .find("const variant")
        .expect("the variant constructor");
    let end = first.find("function w0").expect("the program's own helper");
    assert_eq!(
        end - start,
        1_569,
        "bytes of collection intrinsics per bundle"
    );
    assert_eq!(first.len(), 2_282, "one endpoint bundle");

    let measured = endpoint_duplication(&source, "shared.zd");
    assert_eq!(measured.total, 4_564, "both bundles");
    assert_eq!(
        measured.duplicated, 2_242,
        "bytes the second bundle repeats"
    );
    assert_eq!(
        measured.total - measured.duplicated - first.len(),
        40,
        "bytes of the second bundle that are about the second endpoint"
    );
    assert!(
        measured.percent() > 49.0,
        "BENCHMARKS.md reports two collection endpoints as half a copy of each other; measured \
         {:.1}%",
        measured.percent()
    );
}

/// **And today it is free**, which is the other half of the answer #23 asks
/// for and the half that decides whether anything should be done.
///
/// No example in this repository has two endpoints that share a helper. The
/// three that emit endpoints at all duplicate their generated header and
/// almost nothing else, so the whole cost of per-root emission across every
/// program that exists is a few hundred bytes.
///
/// Bounded above *and* below. Above, because a program that started sharing
/// real code across roots would make the prose in `BENCHMARKS.md` wrong;
/// below, because if this ever reaches zero the header has stopped being
/// written and the measurement is no longer measuring anything.
#[test]
fn the_endpoints_that_exist_today_duplicate_their_header_and_little_else() {
    for (name, bundles, duplicated) in [
        ("examples/guestbook.zd", 3usize, 296usize),
        // Three since #375 gave `tally.zd` a `set … at` write: a command is
        // an endpoint, so the example gained a bundle and another copy of
        // the header. 327 rather than 144 is two headers duplicated instead
        // of one — still the header and almost nothing else, which is what
        // this measures.
        ("examples/tally.zd", 3, 327),
        ("examples/voting-board.zd", 2, 151),
    ] {
        let source = std::fs::read_to_string(repository_path(name)).expect("an example");
        let measured = endpoint_duplication(&source, name);
        assert_eq!(measured.bundles, bundles, "{name}");
        assert_eq!(
            measured.duplicated, duplicated,
            "{name} duplicates {} bytes across its {} endpoint bundles. BENCHMARKS.md says the \
             examples that exist duplicate their header and nothing else; a number that has \
             moved means either the emitter shares more now or an example grew a helper two \
             endpoints reach, and the second is the program #23 asks for.",
            measured.duplicated, measured.bundles
        );
        // The header is what a bundle opens with, and on these programs it
        // is all they have in common.
        assert!(
            measured.common_prefix < 200,
            "{name}'s bundles share a {} byte prefix, which is more than a header",
            measured.common_prefix
        );
    }
}

/// The growth curve behind the table in `BENCHMARKS.md`.
#[test]
#[ignore = "prints the survey behind BENCHMARKS.md; not a gate"]
fn survey_cross_root_duplication() {
    for collections in [false, true] {
        println!(
            "\nendpoints sharing one helper — {}:",
            if collections {
                "a pipeline over a durable list and map"
            } else {
                "arithmetic only"
            }
        );
        println!(
            "{:>8} {:>10} {:>9} {:>10} {:>11} {:>8} {:>9}",
            "helpers", "endpoints", "bundles", "total", "duplicated", "share", "prefix"
        );
        for helpers in [1usize, 4, 16] {
            for endpoints in [1usize, 2, 5, 10, 20, 40] {
                let source = program_with_shared_endpoints(helpers, endpoints, collections);
                let measured = endpoint_duplication(&source, "shared.zd");
                println!(
                    "{helpers:>8} {endpoints:>10} {:>9} {:>10} {:>11} {:>7.1}% {:>9}",
                    measured.bundles,
                    measured.total,
                    measured.duplicated,
                    measured.percent(),
                    measured.common_prefix,
                );
            }
        }
    }

    println!("\nthe examples that emit endpoints today:");
    for name in [
        "examples/guestbook.zd",
        "examples/tally.zd",
        "examples/voting-board.zd",
    ] {
        let source = std::fs::read_to_string(repository_path(name)).expect("an example");
        let measured = endpoint_duplication(&source, name);
        println!(
            "{name:<28} bundles={:<3} total={:<6} duplicated={:<6} ({:.1}%) prefix={}",
            measured.bundles,
            measured.total,
            measured.duplicated,
            measured.percent(),
            measured.common_prefix
        );
    }
}

/// Every runtime module there is, by the name a bundle links it under.
///
/// Listed rather than derived, and that is the point (#145). The gate below
/// is a budget for the runtime *as a whole*, and a budget a list could
/// slip out of would not be one: a new module absent from here would be a
/// new module nothing weighs. Adding a file to `runtime_files` and not to
/// this list fails [`the_runtime_list_is_the_whole_runtime`].
const EVERY_RUNTIME_MODULE: &[&str] = &[
    "runtime/adopt.js",
    "runtime/branch.js",
    "runtime/clock.js",
    "runtime/dom.js",
    "runtime/foreign.js",
    "runtime/keys.js",
    "runtime/list.js",
    "runtime/markup.js",
    "runtime/media.js",
    "runtime/remembered.js",
    "runtime/request.js",
    "runtime/rpc.js",
    "runtime/scene.js",
    "runtime/signal.js",
    "runtime/store.js",
    "runtime/vector.js",
    "runtime/viewport.js",
    "runtime/wire.js",
];

/// The whole runtime's ceiling.
///
/// **Measured, and deliberately not Swift's number.** The first draft of
/// this made the tidier claim — that every line of machinery this language
/// has weighs less than the 73 kB Swift emits for a program that does
/// nothing — and the measurement refused it: the runtime directory is
/// 133 kB. The claim was rhetoric, and it was also comparing the wrong
/// things. Swift's 73 kB is what **one program** ships. This is every
/// module there is, and no program links every module.
///
/// The per-program figures are the ones that carry the comparison, and
/// they are gated above: a null program links `signal.js` and `dom.js`,
/// and `the_null_program_is_a_fraction_of_swifts` holds that under a third
/// of Swift's. A program that also talks to a server and a store links
/// about 64 kB, still under it.
///
/// So this budget does a different job, and it is the job #145 asked for:
/// **it weighs the modules no per-program gate weighs.** It is the current
/// figure plus room for ordinary work — near enough to be reached by real
/// growth, far enough that a normal change fits. It is a tripwire, not a
/// claim, and the number moves when there is a reason it should rather
/// than whenever it is inconvenient.
const WHOLE_RUNTIME_CEILING: usize = 152_000;

// Raised once, for #208's third emission mode. What grew is `adopt.js`
// (5,892 bytes) and `branch.js`'s own header, `branch.js` otherwise being
// bytes moved out of `dom.js` rather than added. The move is why the
// *per-program* figures went the other way — a null program ships 3,160
// bytes fewer than it did — and this budget is the one that has to notice
// that the sum still went up. The reserve is the same size it was before:
// about 6,600 bytes.

/// **Every module is weighed, not just the two a null program links.**
///
/// The gates above are about a null program, so they bound `signal.js` and
/// `dom.js` and nothing else — and the file argues, correctly, that moving
/// bytes into an optional module is a legitimate way to ship less. The
/// consequence is that `list.js`, `scene.js`, `wire.js` and the rest have
/// been unweighed: a program that draws pays for `scene.js`, and no test
/// has ever had an opinion about how large `scene.js` may be.
///
/// That is the hole #145 names. The split is still the right design and
/// this does not undo it — a program links what it uses and no more, which
/// `a_null_program_links_two_runtime_files` pins. What this adds is that
/// the *sum* has a ceiling, so a module nobody's program gates cannot grow
/// without anybody noticing.
#[test]
fn the_whole_runtime_stays_inside_its_budget() {
    let set: std::collections::BTreeSet<&'static str> =
        EVERY_RUNTIME_MODULE.iter().copied().collect();
    let bytes = linked_runtime_bytes_in(&set, zdc_codegen::Mode::Release);
    assert!(
        bytes < WHOLE_RUNTIME_CEILING,
        "the whole runtime is {bytes} bytes against a {WHOLE_RUNTIME_CEILING} byte \
         budget.\n\n\
         This bounds the modules no per-program gate weighs — `scene.js`, `list.js`, \
         `wire.js` and the rest. Raising it is allowed and is a decision: say in the \
         commit what grew and why the language is better for it. The alternative is \
         to split the module so the programs that do not use what you added stop \
         paying for it, which is what `list.js`, `foreign.js` and `markup.js` already \
         are."
    );
}

/// The same reserve the null-program gate keeps, for the same reason: an
/// inequality says nothing until it says everything.
#[test]
fn the_whole_runtime_keeps_room_to_warn_before_it_fails() {
    let set: std::collections::BTreeSet<&'static str> =
        EVERY_RUNTIME_MODULE.iter().copied().collect();
    let bytes = linked_runtime_bytes_in(&set, zdc_codegen::Mode::Release);
    let headroom = WHOLE_RUNTIME_CEILING.saturating_sub(bytes);
    assert!(
        headroom >= HEADROOM_FLOOR,
        "the whole runtime is {bytes} bytes against {WHOLE_RUNTIME_CEILING}, leaving \
         {headroom} — under the {HEADROOM_FLOOR} this suite keeps in reserve.\n\n\
         This is not a failure of whatever change you just made. It is the runtime \
         having grown to where the next change cannot fit, and it is the warning the \
         budget above exists to give one change earlier."
    );
}

/// A module the bundler can link but this suite does not weigh is a module
/// with no budget, which is the whole failure mode [`EVERY_RUNTIME_MODULE`]
/// exists to prevent.
#[test]
fn the_runtime_list_is_the_whole_runtime() {
    for name in EVERY_RUNTIME_MODULE {
        let set: std::collections::BTreeSet<&'static str> = std::iter::once(*name).collect();
        assert!(
            linked_runtime_bytes_in(&set, zdc_codegen::Mode::Release) > 0,
            "`{name}` is listed here but the bundler does not know it — either it was \
             renamed or it never existed, and either way this list is weighing a file \
             that is not shipped"
        );
    }
    let all: std::collections::BTreeSet<&'static str> =
        EVERY_RUNTIME_MODULE.iter().copied().collect();
    assert_eq!(
        all.len(),
        EVERY_RUNTIME_MODULE.len(),
        "a name is in the list twice, which weighs it twice"
    );
    // The other direction, and the one that matters: a module the emitter
    // can link and this list has never heard of.
    let known = zdc_codegen::runtime_files(&all, zdc_codegen::Mode::Release);
    assert_eq!(
        known.len(),
        EVERY_RUNTIME_MODULE.len(),
        "`runtime_files` returned a different number of sources than the list asked \
         for, which means the two disagree about what the runtime is"
    );
}
