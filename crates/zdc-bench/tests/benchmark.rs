//! The benchmark gate.
//!
//! §14A.4: *"Regressions in these numbers are build failures, not
//! observations."* This file is where that sentence is enforced. Three kinds
//! of assertion, in order of what they protect:
//!
//! 1. **Every arm renders the same DOM.** Without this the numbers mean
//!    nothing, because an arm can always be fast by being wrong — which is
//!    exactly the failure §16.6 found in the previous reconciler, where
//!    update and swap were silent no-ops.
//! 2. **The claims in §14A and §16.1 hold**, expressed as inequalities with
//!    headroom, so a code generator that stops paying for itself fails the
//!    build rather than being noticed later.
//! 3. **The committed table still matches the code.** Any change to any
//!    number, in any direction, fails until `BENCHMARKS.md` is regenerated.
//!
//! The workload takes about two minutes and every test in this binary shares
//! one run of it.

use std::sync::OnceLock;

use zdc_bench::{bundle_sizes, generated_section, Report, END_MARKER, START_MARKER};

fn report() -> &'static Report {
    static REPORT: OnceLock<Report> = OnceLock::new();
    REPORT.get_or_init(zdc_bench::run)
}

/// The reorder measurement, shared by every test in this binary for the
/// reason the workload above is: it is a second run of the runtime and it
/// costs about as much again.
fn reorder() -> &'static Report {
    static REPORT: OnceLock<Report> = OnceLock::new();
    REPORT.get_or_init(zdc_bench::run_reorder)
}

/// The reconciler that ships, and the one it replaced.
const LIS: &str = "lis";
const CURSOR: &str = "cursor";

const ZD: &str = "zd-positional";
const IDENTITY: &str = "zd-identity";
const DIRECT: &str = "direct";
const VANILLA: &str = "vanilla";
const TUNED: &str = "vanilla-tuned";

const CREATE_10K: &str = "create 10,000 rows";
const CREATE_1K: &str = "create 1,000 rows";

/// Nothing below means anything unless the arms agree on what they rendered.
///
/// The digest is the element tree, its text, and its attributes, with
/// `class` compared as the token set a browser treats it as. An arm that
/// skipped an update, moved the wrong node, or left a stale row behind
/// fails here first.
#[test]
fn every_arm_renders_the_same_dom_at_every_step() {
    let report = report();
    let arms = report.arms();
    assert_eq!(arms.len(), 5, "expected five arms, got {arms:?}");

    for step in report.steps() {
        let first = report.find(arms[0], step);
        for arm in &arms[1..] {
            let other = report.find(arm, step);
            assert_eq!(
                other.get("digest"),
                first.get("digest"),
                "after `{step}`, `{arm}` rendered a different DOM from `{}`. \
                 One of them is wrong; the counts are meaningless until they agree.",
                arms[0]
            );
            assert_eq!(
                other.get("rows"),
                first.get("rows"),
                "after `{step}`, `{arm}` has a different row count from `{}`",
                arms[0]
            );
        }
    }
}

/// The workload really did render what it claims to have rendered.
///
/// A digest comparison passes trivially if every arm renders nothing.
#[test]
fn the_workload_rendered_the_rows_it_says_it_did() {
    let report = report();
    let arms = report.arms();
    assert_eq!(arms.len(), 5, "five arms are measured, got {arms:?}");
    for arm in arms {
        assert_eq!(report.find(arm, CREATE_1K).get("rows"), 1_000);
        assert_eq!(report.find(arm, CREATE_10K).get("rows"), 10_000);
        assert_eq!(
            report.find(arm, "append 1,000 to 10,000").get("rows"),
            11_000
        );
        assert_eq!(report.find(arm, "remove a row").get("rows"), 999);
        assert_eq!(report.find(arm, "clear 11,000 rows").get("rows"), 0);
    }
}

/// §16.1: template cloning against the direct emission it was chosen over.
///
/// The spec claims *"4.2× fewer DOM API crossings"* on this row shape.
/// Measured here it is 3.1×, which is a real advantage and not the claimed
/// one — see `BENCHMARKS.md`. The gate is set at 2× rather than at the
/// measured 3.1× because the ratio depends on how many holes and handlers a
/// row has, and a benchmark that fails when someone adds an attribute to the
/// row is a benchmark people delete. At 2× the architectural claim is still
/// falsifiable: template cloning that stopped being worth its complexity
/// could not pass it.
#[test]
fn template_cloning_halves_the_dom_crossings_of_direct_emission() {
    let report = report();
    for step in [CREATE_1K, CREATE_10K] {
        let template = report.find(ZD, step).get("crossings");
        let direct = report.find(DIRECT, step).get("crossings");
        assert!(
            template * 2 <= direct,
            "`{step}`: template cloning made {template} DOM crossings and direct emission \
             made {direct}. §16.1 chose template cloning over direct emission on this \
             number; at less than 2× the choice no longer pays for itself."
        );
    }
}

/// §16.1: *"1,000 fewer effect allocations"* at N=1,000. Measured exactly.
///
/// One effect per hole against one per node. The gate is the spec's own
/// claim, stated per row so it holds at both sizes.
#[test]
fn template_cloning_allocates_one_fewer_effect_per_row() {
    let report = report();
    for step in [CREATE_1K, CREATE_10K] {
        let rows = report.find(ZD, step).get("rows");
        let template = report.find(ZD, step).get("reactive.effect");
        let direct = report.find(DIRECT, step).get("reactive.effect");
        assert!(
            direct - template >= rows,
            "`{step}`: {template} effects against direct emission's {direct} over {rows} rows. \
             §16.1 claims at least one fewer effect per row."
        );
        assert!(
            template <= rows * 3,
            "`{step}`: {template} effects for {rows} rows. The row has three holes, so three \
             effects per row is the ceiling; more means a binding is being created that the \
             emitter does not need."
        );
    }
}

/// §14A.2: *"A hand-tuned vanilla-JS micro-app will beat us. This is the one
/// comparison we lose."*
///
/// It still does, and this test says so out loud. If it ever fails because
/// the emitter got better, the spec is what needs changing — which is the
/// point of writing the claim down as an assertion.
#[test]
fn hand_tuned_vanilla_is_still_the_floor() {
    let report = report();
    let tuned = report.find(TUNED, CREATE_10K).get("crossings");
    let template = report.find(ZD, CREATE_10K).get("crossings");
    assert!(
        tuned < template,
        "hand-tuned vanilla made {tuned} DOM crossings and the emitted code made {template}. \
         §14A.2 states plainly that hand-tuned vanilla beats us; if that has stopped being \
         true, the spec is now wrong and should be corrected rather than this test relaxed."
    );
    // 2.5×, against 1.75× measured. The ceiling is here to catch the loss
    // widening by an order of magnitude, not to fail the build over one
    // extra per-row write — the golden table already catches that exactly.
    assert!(
        template * 2 <= tuned * 5,
        "the emitted code made {template} DOM crossings against hand-tuned vanilla's {tuned}. \
         §14A.2 concedes the loss but calls it a micro-app effect that does not generalise; \
         past 2.5× at 10,000 rows it has generalised, and the concession is understated."
    );
}

/// What §14A.2 does *not* claim, and the measurement supports: emitted code
/// beats hand-written vanilla that is not hand-tuned — the node-by-node
/// style js-framework-benchmark's own `vanillajs` entry uses.
#[test]
fn emitted_code_beats_vanilla_written_node_by_node() {
    let report = report();
    let vanilla = report.find(VANILLA, CREATE_10K).get("crossings");
    let template = report.find(ZD, CREATE_10K).get("crossings");
    assert!(
        template * 2 <= vanilla,
        "emitted {template} DOM crossings against node-by-node vanilla's {vanilla}; \
         template cloning should be at least twice as frugal as building each node by hand."
    );
}

/// §16.6's cost table, which is the honest part of the interim keying
/// decision. Both halves are gated: what identity keying buys, and what
/// positional keying costs until `record … unique` lands.
#[test]
fn the_keying_costs_are_the_ones_the_spec_admits_to() {
    let report = report();

    // Identity keying: a removal is one call and nothing else moves. This is
    // what R1's two-pass retire bought (994 moves → 0).
    let removal = report.find(IDENTITY, "remove a row");
    assert!(
        removal.get("crossings") <= 2,
        "removing one row under identity keying cost {} DOM crossings; the two-pass retire \
         in §16.2 R1 makes it one `removeChild` and no moves.",
        removal.get("crossings")
    );

    // Identity keying: a two-row swap moved 997 nodes at N=1,000 until the
    // longest-increasing-subsequence reconciler §16.10 scheduled landed.
    // It is two now, and two is the minimum — the two rows that changed
    // places. Pinned exactly rather than ranged: there is no row shape or
    // list size this number depends on, so any other value is a defect
    // rather than a drift, and `reordering_moves_the_fewest_rows_it_can`
    // is what says so at three sizes and in four shapes.
    let swap = report.find(IDENTITY, "swap two rows");
    assert_eq!(
        swap.get("cross.insertBefore"),
        2,
        "a two-row swap under identity keying moved {} nodes. Exchanging two rows needs two \
         moves; more means the reconciler stopped computing a minimal move set.",
        swap.get("cross.insertBefore")
    );

    // Positional keying: identity is the slot, so a removal rewrites every
    // row after it. This is the number that makes `record … unique` urgent
    // rather than nice to have.
    let shifted = report.find(ZD, "remove a row");
    assert!(
        shifted.get("crossings") >= 1_000,
        "removing one row under positional keying cost only {} DOM crossings. That would be \
         good news and it would mean §16.6's account of positional keying is out of date.",
        shifted.get("crossings")
    );
    assert!(
        shifted.get("crossings") <= 4_000,
        "removing one row under positional keying cost {} DOM crossings, up from the 2,986 \
         measured. Positional keying is already the worst number in this suite.",
        shifted.get("crossings")
    );
}

/// Nothing below means anything unless both reconcilers agree on what they
/// rendered. A reconciler that moved fewer nodes by leaving the list in the
/// wrong order would pass every count in this file and fail here.
#[test]
fn both_reconcilers_leave_the_list_in_the_same_order() {
    let reorder = reorder();
    let steps = reorder.steps();
    assert_eq!(steps.len(), 12, "four shapes at three sizes: {steps:?}");
    for step in steps {
        let lis = reorder.find(LIS, step);
        let cursor = reorder.find(CURSOR, step);
        assert_eq!(
            lis.get("digest"),
            cursor.get("digest"),
            "after `{step}` the two reconcilers rendered different orders. \
             One of them is wrong; the move counts are meaningless until they agree."
        );
        assert_eq!(lis.get("rows"), cursor.get("rows"), "after `{step}`");
    }
}

/// §16.10, issue #207: *"Identity-keyed reordering is O(n) moves until the
/// LIS reconciler lands."* It has landed, and this is the measurement that
/// says so — the exact move set every shape costs, at every size.
///
/// Exact rather than ranged. A minimal move set is a combinatorial fact
/// about the permutation and not a property of the row shape, the engine
/// or the list's contents, so there is nothing here for headroom to
/// absorb: any other number is a defect.
#[test]
fn reordering_moves_the_fewest_rows_it_can() {
    let reorder = reorder();
    // (shape, N, minimal moves, what the cursor walk cost)
    let expected = [
        // Two rows change places, so two rows move — at every size. This
        // is the row §16.6 measured at 997 and scheduled the fix for.
        ("swap two rows", 100, 2, 97),
        ("swap two rows", 1000, 2, 997),
        ("swap two rows", 5000, 2, 4997),
        // One row is out of place, and the cursor walk already found this
        // one: it is here so the win is not overstated.
        ("move the last row to the front", 100, 1, 1),
        ("move the last row to the front", 1000, 1, 1),
        ("move the last row to the front", 5000, 1, 1),
        // A permutation is not all a real update is. Two moves for the
        // swap, one for the row appended at the end, one for the row that
        // shifted into the gap the removal left.
        ("remove one, add one, swap two", 100, 4, 98),
        ("remove one, add one, swap two", 1000, 4, 998),
        ("remove one, add one, swap two", 5000, 4, 4998),
        // The worst case, and the reason this is a minimal move set rather
        // than a small one: a reversal has no increasing subsequence longer
        // than one row, so n - 1 moves is optimal and there is nothing to
        // save. An implementation that beat this would be wrong.
        ("reverse the whole list", 100, 99, 99),
        ("reverse the whole list", 1000, 999, 999),
        ("reverse the whole list", 5000, 4999, 4999),
    ];
    for (shape, size, minimal, walked) in expected {
        let step = format!("{shape} at N={size}");
        assert_eq!(
            reorder.find(LIS, &step).get("moves"),
            minimal,
            "`{step}` should cost {minimal} moves"
        );
        assert_eq!(
            reorder.find(CURSOR, &step).get("moves"),
            walked,
            "`{step}` cost the cursor walk a different number than the {walked} recorded; \
             the before column in BENCHMARKS.md is then stale"
        );
    }
}

/// The order-of-growth claim, stated as the only thing a benchmark can
/// honestly say about one: the count stopped depending on the list.
///
/// A single size cannot distinguish O(1) from O(n) — 2 moves out of 1,000
/// and 2 moves out of 2 are the same number. Three sizes spanning 50× can:
/// the reconciler that ships costs the same at all three, and the one it
/// replaced costs fifty times more at the largest than at the smallest.
#[test]
fn the_cost_of_a_reorder_no_longer_grows_with_the_list() {
    let reorder = reorder();
    for shape in ["swap two rows", "remove one, add one, swap two"] {
        let at = |arm: &str, size: usize| {
            reorder
                .find(arm, &format!("{shape} at N={size}"))
                .get("moves")
        };
        assert_eq!(
            (at(LIS, 100), at(LIS, 1000)),
            (at(LIS, 1000), at(LIS, 5000)),
            "`{shape}` cost the LIS reconciler {}, {} and {} moves at N=100, 1,000 and 5,000. \
             The move set for this shape is the same size whatever the list's length, so a \
             count that varies with it means the reconciler is walking the list rather than \
             the moves.",
            at(LIS, 100),
            at(LIS, 1000),
            at(LIS, 5000)
        );
        assert!(
            at(CURSOR, 5000) >= at(CURSOR, 100) * 40,
            "`{shape}` cost the cursor walk {} moves at N=100 and {} at N=5,000. It is the \
             linear arm; if it has stopped being linear it is no longer the algorithm this \
             change replaced and the before column is measuring something else.",
            at(CURSOR, 100),
            at(CURSOR, 5000)
        );
    }
}

/// The one number where a general reconciler is beaten by an idiom rather
/// than by tuning: emptying a list.
///
/// A framework retires rows one at a time; `replaceChildren()` is one call
/// whatever the length. Gated so the O(n) teardown is visible rather than
/// forgotten.
#[test]
fn clearing_a_list_is_linear_for_every_reactive_arm() {
    let report = report();
    for arm in [ZD, IDENTITY, DIRECT] {
        let clear = report.find(arm, "clear 11,000 rows");
        assert_eq!(
            clear.get("cross.removeChild"),
            11_000,
            "`{arm}` cleared 11,000 rows with {} `removeChild` calls",
            clear.get("cross.removeChild")
        );
    }
    for arm in [VANILLA, TUNED] {
        assert_eq!(
            report.find(arm, "clear 11,000 rows").get("crossings"),
            1,
            "`{arm}` should clear the list with one `replaceChildren()`"
        );
    }
}

/// §14A.4 also asks for bundle size. Bytes as shipped: there is no minifier
/// in the pipeline, so these are the real numbers and not a projection.
///
/// The ceilings are round numbers roughly 50% above what is emitted today.
/// They are not a target; they exist so that a code generator that starts
/// emitting a helper per node, or a runtime that grows a framework inside
/// it, fails the build.
#[test]
fn the_emitted_bundle_and_the_runtime_stay_small() {
    for size in bundle_sizes() {
        assert!(
            size.client_js <= 2_048,
            "`{}` emitted {} bytes of client.js; the ceiling is 2,048",
            size.name,
            size.client_js
        );
    }
    let runtime = zdc_runtime::SIGNAL_JS.len() + zdc_runtime::DOM_JS.len();
    assert!(
        runtime <= 24_576,
        "the runtime a bundle links against is {runtime} bytes; the ceiling is 24,576. \
         It is unminified and heavily commented, so this is not a byte-count contest — \
         it is a check that no framework has grown inside it."
    );
}

/// The committed table is generated from the measurements, so a number in
/// the repository that disagrees with the code is a build failure.
///
/// Regenerate with `ZDC_BLESS=1 cargo test -p zdc-bench`.
#[test]
fn the_committed_results_match_the_measurements() {
    let path = zdc_bench::repository_path("BENCHMARKS.md");
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let generated = generated_section(report(), reorder());

    let start = committed
        .find(START_MARKER)
        .unwrap_or_else(|| panic!("{} has no `{START_MARKER}`", path.display()))
        + START_MARKER.len();
    let end = committed
        .find(END_MARKER)
        .unwrap_or_else(|| panic!("{} has no `{END_MARKER}`", path.display()));
    let existing = committed[start..end].trim_matches('\n');

    if existing == generated.trim_matches('\n') {
        return;
    }

    if std::env::var_os("ZDC_BLESS").is_some() {
        let rewritten = format!(
            "{}{START_MARKER}\n\n{}\n{}",
            &committed[..start - START_MARKER.len()],
            generated.trim_matches('\n'),
            &committed[end..]
        );
        std::fs::write(&path, rewritten).expect("rewriting BENCHMARKS.md");
        panic!("BENCHMARKS.md has been regenerated. Review the diff and commit it.");
    }

    panic!(
        "BENCHMARKS.md no longer matches the measurements. §14A.4 makes that a build failure, \
         not an observation. Inspect the change, and if it is intended run \
         `ZDC_BLESS=1 cargo test -p zdc-bench` to regenerate the table.\n\n\
         committed:\n{existing}\n\nmeasured:\n{generated}"
    );
}
