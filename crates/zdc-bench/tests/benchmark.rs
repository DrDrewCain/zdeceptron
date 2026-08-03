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

    // Identity keying: a two-row swap is O(n) moves until the LIS
    // reconciler lands. §16.6 measures 997 at N=1,000, accepts it, and
    // schedules the fix. The ceiling keeps it from quietly getting worse.
    let swap = report.find(IDENTITY, "swap two rows");
    assert!(
        (900..=1_100).contains(&swap.get("cross.insertBefore")),
        "a two-row swap under identity keying moved {} nodes. §16.6 measures 997 at N=1,000 \
         and schedules the longest-increasing-subsequence fix; a different number means the \
         reconciler changed and the spec's figure is stale.",
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
    let generated = generated_section(report());

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
