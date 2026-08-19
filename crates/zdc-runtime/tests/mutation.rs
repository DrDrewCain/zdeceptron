//! Change the runtime, and see whether any test notices.
//!
//! `scripts/check-vacuous-tests.py` is the static half of this question.
//! It reads test source for two shapes that cannot fail — an
//! `assert!(a || b)` with an unconditional arm, and a loop over something
//! that can be empty — and it was written after six such tests were found
//! in one night. It is a good gate and it is still the first one to run.
//! What it cannot see is a test that has every shape of a real test and
//! is nonetheless measuring something other than what it claims.
//!
//! Four of those were found in this repository inside a day, all four
//! green the whole time. One located a function's end by the first `}` in
//! column 0, and minification moved the brace. One matched a comment to a
//! call by adjacency, and `cargo fmt` put a line between them. One
//! attributed a latency to the emitter by subtracting two runs that
//! compile different programs. **And one asserted that a page mounted,
//! against a page that threw on its first line, because the harness
//! linked a hand-written module list that had stopped being the whole
//! list.** No syntactic rule catches any of them, because there is
//! nothing wrong with the syntax. What catches them is changing the thing
//! under test and asking whether anything goes red.
//!
//! # What is mutated, and why only this
//!
//! Mutation testing a Rust workspace of this size means recompiling for
//! every mutant, and would not finish in CI. The runtime modules are the
//! one part of the compiler's output that is **data** — `MODULES` is a
//! list of `&'static str`, and every suite that runs them takes the source
//! as an argument — so a mutant here is a string edit and a fresh
//! `boa` context, with no build in the loop. That is also exactly where
//! the fourth defect above lived: `MODULES` itself was three modules short
//! of what a bundle writes while this file was being written, and the
//! commit before this one is the repair.
//!
//! So: the runtime's JavaScript, mutated in memory, run against the
//! JavaScript suites this crate already owns. Nothing else. A harness that
//! mutates one thing and is believed beats a framework that mutates
//! everything and is not, and the honest boundary is written down in
//! `UNREACHED` below rather than left for a reader to discover.
//!
//! # Two tiers
//!
//! [`every_module_is_reached_by_a_suite_here`] runs by default. It poisons
//! each module in turn — every function in it throws when called — and
//! asserts some suite fails. That is the question the fourth defect
//! answers wrongly: *is this module run by anything at all?* A module no
//! suite reaches is named in `UNREACHED`, with the reason, so the set can
//! only shrink silently and never grow silently. It found one on its first
//! run: `markup.js` holds the only assignment to `innerHTML` in the
//! runtime, two suites here load it, and no case in either calls it.
//!
//! [`no_mutation_of_the_runtime_goes_unnoticed`] is `#[ignore]`d and has a
//! CI job of its own, the way the browser tests do. It is the full sweep:
//! one mutant per function and one per comparison, 236 in all, of which 58
//! survive. Every survivor is printed by name, and the *set* of them is
//! gated against `SURVIVORS`, which explains them group by group. A
//! survivor is a finding about the tests rather than a failure of this
//! file, so the gate is that the set has not moved and not that it is
//! empty — a harness that demanded zero would be deleted the first time
//! somebody needed to ship.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use boa_engine::{Context, Source};

/// How many times a loop in a mutated module may go round before the
/// engine stops it.
///
/// **A mutation harness without this does not finish.** The first sweep
/// written here was abandoned by hand after twenty minutes with no output:
/// ` < ` became ` <= ` in a reconciler's loop condition, and the mutant did
/// exactly what it was asked to and never returned. That is not a rare
/// mutant, it is a whole operator's worth of them — a boundary flip is a
/// non-termination generator — so the bound is part of the design and not
/// a safety net.
///
/// A bound rather than a timeout, for the reason `Sandbox` gives for the
/// same choice: the same mutant has to die on a slow machine and a fast
/// one, or a gate's answer depends on how busy the runner was. It is two
/// orders of magnitude below `Sandbox`'s, because nothing a suite does
/// legitimately loops this far and the sweep pays this cost once per
/// runaway mutant.
const LOOP_BUDGET: u64 = 500_000;

// --- the suites, and what each of them links ----------------------------

/// The `test`/`assert` shim the suites are written against.
///
/// A fourth copy of the one in `render.rs`, `reactivity.rs` and `wire.rs`,
/// and deliberately a copy: this file's whole job is to be an independent
/// answer to "does that suite still work", and a harness shared with the
/// suites under examination would fail with them. It is twenty lines of
/// obviously-correct JavaScript, which is the reason the other three
/// tolerate the duplication too.
const HARNESS: &str = r#"
const __results = [];
function test(name, fn) {
  try {
    fn();
    __results.push({ name, ok: true, message: '' });
  } catch (e) {
    __results.push({ name, ok: false, message: String(e && e.message ? e.message : e) });
  }
}
const assert = {
  equal(actual, expected, note) {
    if (!Object.is(actual, expected)) {
      throw new Error(
        (note ? note + ': ' : '') + 'expected ' + JSON.stringify(expected) +
        ', got ' + JSON.stringify(actual)
      );
    }
  },
  ok(value, note) {
    if (!value) throw new Error(note || 'expected a truthy value');
  },
  deepEqual(actual, expected, note) {
    const a = JSON.stringify(actual);
    const b = JSON.stringify(expected);
    if (a !== b) {
      throw new Error((note ? note + ': ' : '') + 'expected ' + b + ', got ' + a);
    }
  },
};
"#;

const REPORT: &str = r#"
__results.map(r => (r.ok ? 'PASS ' : 'FAIL ') + r.name + (r.ok ? '' : ' :: ' + r.message)).join('\n')
"#;

/// One JavaScript suite, and the runtime modules it loads.
///
/// The module list is spelled with the same paths `MODULES` uses, so a
/// name here that is not a module fails immediately in [`source_of`]
/// rather than quietly linking nothing — which is the failure this whole
/// file is about.
struct Detector {
    suite: &'static str,
    source: &'static str,
    modules: &'static [&'static str],
}

/// Every suite in this crate, with the modules `render.rs`, `wire.rs` and
/// `reactivity.rs` hand them.
///
/// This is a second copy of those three files' linkage, and the copy is
/// load-bearing rather than lazy: a mutation harness that imported the
/// suite runner it is examining would inherit its mistakes. The check that
/// keeps the copy honest is not a diff against them — it is
/// [`every_module_is_reached_by_a_suite_here`], which fails when a module
/// is linked by nothing here whatever the other files do.
const DETECTORS: &[Detector] = &[
    Detector {
        suite: "signal.test.js",
        source: include_str!("../runtime/signal.test.js"),
        modules: &["runtime/signal.js"],
    },
    Detector {
        suite: "dom.test.js",
        source: include_str!("../runtime/dom.test.js"),
        modules: &[
            "runtime/signal.js",
            "runtime/dom.js",
            // `when` and `if` live here since the branch dispatchers moved
            // out of `dom.js`. The suite calls them, so without this it
            // fails before a mutant is applied — and a suite that is
            // already red catches every mutation by accident.
            "runtime/branch.js",
            "runtime/markup.js",
            "runtime/list.js",
        ],
    },
    Detector {
        suite: "elements.test.js",
        source: include_str!("../runtime/elements.test.js"),
        modules: &[
            "runtime/signal.js",
            "runtime/dom.js",
            "runtime/branch.js",
            "runtime/markup.js",
            "runtime/list.js",
            "runtime/elements.js",
        ],
    },
    Detector {
        suite: "list.test.js",
        source: include_str!("../runtime/list.test.js"),
        modules: &["runtime/signal.js", "runtime/dom.js", "runtime/list.js"],
    },
    Detector {
        suite: "foreign.test.js",
        source: include_str!("../runtime/foreign.test.js"),
        modules: &["runtime/signal.js", "runtime/foreign.js"],
    },
    Detector {
        suite: "handler.test.js",
        source: include_str!("../runtime/handler.test.js"),
        modules: &["runtime/signal.js", "runtime/dom.js"],
    },
    Detector {
        suite: "keys.test.js",
        source: include_str!("../runtime/keys.test.js"),
        modules: &["runtime/signal.js", "runtime/keys.js"],
    },
    Detector {
        suite: "media.test.js",
        source: include_str!("../runtime/media.test.js"),
        modules: &["runtime/signal.js", "runtime/media.js"],
    },
    Detector {
        suite: "clock.test.js",
        source: include_str!("../runtime/clock.test.js"),
        modules: &["runtime/signal.js", "runtime/clock.js"],
    },
    Detector {
        suite: "wire.test.js",
        source: include_str!("../runtime/wire.test.js"),
        modules: &["runtime/wire.js"],
    },
];

/// A module a bundle ships that no suite in this crate runs, and why.
///
/// Every entry is a hole, not an exemption: what it says is that a mutation
/// of that module cannot be caught here, so this file has nothing to say
/// about it. The second field is where its coverage does live, so a reader
/// can go and check whether that place would notice a change — which is
/// the question this whole file exists to make askable.
///
/// The list is checked in both directions. A module in it that a suite has
/// since started running is an error, so it cannot outlive its cause.
const UNREACHED: &[(&str, &str)] = &[
    (
        // Found by this file, and the least expected of the six: unlike
        // the others `markup.js` *is* linked, by `dom.test.js` and by
        // `elements.test.js` both, and neither of them ever calls it. Two
        // suites load the one function in the runtime that assigns
        // `innerHTML` and no case in either exercises it — which is the
        // difference between linking a module and testing one, and the
        // reason poisoning is the question this file asks rather than
        // reading the load lists.
        "runtime/markup.js",
        "driven from `zdc-codegen/tests/markup.rs`, which renders a `Prose` \
         through a compiled program and reads the parsed nodes back out of \
         the shim — thorough, and one crate away from here",
    ),
    (
        // `scene.js` and `vector.js` arrived together with the drawing
        // vocabulary, and neither has a suite in this crate. Named here
        // rather than given one, because what would exercise them is a
        // canvas: `scene.js` asks for a WebGPU context, falls back to WebGL
        // and then to Canvas 2D, and `dom-shim.js` models none of the
        // three. A suite here could only check that the module parses.
        "runtime/scene.js",
        "driven from `zdc-codegen/tests/element_parity.rs`, which compiles a \
         `Scene` and holds the emitted shape to the element table; the \
         renderer itself needs a browser, and `zdc-cli/tests/browser.rs` is \
         where one is",
    ),
    (
        // Adoption is a claim about two *different* renderers agreeing — a
        // Rust serialiser's bytes and a browser parser's tree — so a suite
        // against the shim could only check that this module agrees with
        // the shim, which is the half that was never in doubt.
        "runtime/adopt.js",
        "driven from `zdc-cli/tests/browser.rs`, where \
         `a_prerendered_page_is_adopted_by_the_client_rather_than_rebuilt` \
         serves a built page to a real browser and counts how many of the \
         elements the build wrote are still the same nodes afterwards",
    ),
    (
        // Both became visible to this gate the moment `MODULES` learned
        // about them — which is what that list being checked rather than
        // remembered buys: a module nothing runs used to be invisible here
        // too.
        "runtime/viewport.js",
        "driven from `zdc-codegen/tests/clock.rs`, which compiles a program \
         reading `from scroll` and drives the hoisted cell through the shim",
    ),
    (
        "runtime/prerender.js",
        "not shipped to a browser at all: it is the build host's own DOM \
         walk, run by `zdc-codegen`'s `prerender.rs` on every example with a \
         first paint, and `clock.rs` reads the painted document back out",
    ),
    (
        "runtime/vector.js",
        "linked only by a program the maths prelude reaches, and exercised \
         through it: `zdc-codegen/tests/library.rs` runs `dot`, `magnitude` \
         and the rest as compiled programs and reads the answers back out",
    ),
    (
        "runtime/request.js",
        "driven from `zdc-codegen/tests/outbound.rs`, through a compiled \
         program rather than a suite of its own",
    ),
    (
        "runtime/rpc.js",
        "driven from `tests/transport_contract.rs` here and from \
         `zdc-codegen/tests/failure_code.rs`, both through the Rust API",
    ),
    (
        "runtime/store.js",
        "driven from `tests/transport_contract.rs` here and from \
         `zdc-codegen`'s `live_context`, both through the Rust API",
    ),
    (
        "runtime/remembered.js",
        "nothing under a plain `cargo test` runs it; \
         `zdc-cli/tests/browser.rs::a_remembered_value_survives_a_reload_in_a_real_browser` \
         does, in the `#[ignore]`d job that has a browser",
    ),
];

/// Mutants no suite here catches, counted by `<module>::<operator>`, with
/// the reason each group survives.
///
/// Same contract as `WAIVED` in `scripts/check-vacuous-tests.py`, in both
/// directions: a group that grows fails, and a group that empties fails,
/// so an entry cannot outlive the gap it records.
///
/// **Counted per group rather than listed per mutant, and that is a
/// deliberate weakening.** A key would have to be
/// `<module>::<operator>::<occurrence>`, and an occurrence index moves
/// whenever an *earlier* one is added — so a single new `===` near the top
/// of `elements.js` would renumber fifteen entries and turn this list into
/// something a contributor regenerates rather than reads. The cost of
/// counting instead is real and worth naming: within one group, a mutant
/// that gains coverage and another that loses it cancel out and this list
/// does not notice. The full keys are printed on every run, which is where
/// to look when a count moves by one and the reason is not obvious.
///
/// The reasons fall into four kinds, and the difference between them
/// matters more than the numbers:
///
/// * **Covered one crate away.** `zdc-runtime`'s suites are not where most
///   of this coverage lives; `zdc-codegen` drives the same modules through
///   compiled programs, and `zdc-bench` gates what a reconciliation costs.
///   Those groups are a division of labour, and the largest share by far.
/// * **A message, not a decision.** Mutating the text an assertion throws
///   leaves a suite that checks *that* it threw entirely happy.
/// * **Equivalent.** The mutated program computes the same thing. No test
///   can catch these and none should have to.
/// * **A genuine hole**, marked GAP. Six mutants, in three groups, and
///   they are the point of this file. GAP is a claim about *this crate* —
///   no suite here notices — plus a search of the rest of the workspace
///   that turned up nothing that plainly owns the behaviour. That is
///   weaker than "nothing anywhere would notice", deliberately: this
///   harness cannot run `zdc-codegen`'s tests and must not pretend to have.
///   Each one names the behaviour precisely enough for a reader to check.
const SURVIVORS: &[(&str, usize, &str)] = &[
    (
        "runtime/branch.js::and-to-or",
        1,
        "GAP, and the one `dom.js` recorded until the dispatchers moved here: \
         `ifInto`'s `onCleanup(() => disposeBranch && disposeBranch())`, which \
         is the *outer* disposal — what happens to the showing branch's \
         bindings when the `if` itself is torn down, not when its condition \
         flips. The flip is inside the effect and dies. This one leaves an \
         effect subscribed to a signal, running against detached nodes, which \
         is a leak with no symptom",
    ),
    (
        "runtime/branch.js::equal-to-unequal",
        3,
        "two different things. Two are the adoption comparison, once in \
         `whenInto` and once in `ifInto`: `start.nodeValue === mark` is what \
         decides whether the served region is claimed or dropped, and only a \
         prerendered document reaches it. The suites that serve one are \
         `zdc-cli/tests/browser.rs` and the prerender tests, both a crate \
         away. The third is inside `read` and is not measurable here at all — \
         see the entry below",
    ),
    (
        "runtime/branch.js::unreached-function",
        1,
        "`read`, and this one is a limit of the harness rather than of the \
         suites: replacing its body with a throw fails three cases in \
         `--test render`. `dom.js`, `branch.js` and `list.js` each declare a \
         byte-identical top-level `read`, kept separate so a program links \
         neither module it does not use. `report_of` flattens every module a \
         suite links into one scope, the last declaration hoists over the \
         rest, and a mutant applied to this copy is undone before it runs. \
         Harmless while the three agree; #394 is why that is not something to \
         rely on",
    ),
    (
        "runtime/clock.js::and-to-or",
        1,
        "equivalent while the suite installs a `performance`: `A && A.now` and \
         `A || A.now` agree whenever `A` is defined, and the case where they \
         differ is a host that has none, which no suite here creates",
    ),
    (
        "runtime/clock.js::unreached-function",
        2,
        "`steppingMs` and `steppingFrame`, the two sources behind `every … ms` \
         and `every frame`. `clock.test.js` is about the scheduler — it \
         installs a `performance` and a fake timer and asserts on what the \
         scheduler does with them — and constructs neither source. \
         `zdc-codegen/tests/clock.rs` builds both a crate away, through the \
         compiler that emits them",
    ),
    (
        "runtime/dom.js::and-to-or",
        1,
        "the inline-`style` branch of `props`, which no case here passes and \
         `zdc-codegen/tests/element_parity.rs` covers a crate away by \
         comparing whole trees. This group held a second entry until the \
         branch dispatchers moved to `branch.js`, and the GAP moved with \
         them — it is recorded under `runtime/branch.js::and-to-or` now",
    ),
    (
        "runtime/dom.js::equal-to-unequal",
        2,
        "the same inline-`style` branch: `elements.js` is what produces that \
         shape, and the trees it produces are compared against the compiler's \
         own table in `zdc-codegen/tests/element_parity.rs`",
    ),
    (
        "runtime/dom.js::or-to-and",
        4,
        "GAP: the empty-value branches — an attribute set to `false`, `null` or \
         `undefined`, a `null` child, and twice over a text binding whose value \
         is absent. Reversing the last of those renders the string `null` where \
         a reader should see nothing, and every suite in this crate stays green",
    ),
    (
        "runtime/dom.js::unreached-function",
        1,
        "`mount` is what generated code calls to put a view in the document, \
         and no suite here calls it: `dom.test.js` builds trees and reads them \
         back without ever mounting one. `zdc-cli/tests/browser.rs` mounts for \
         real, in the job that needs a browser",
    ),
    (
        "runtime/elements.js::and-to-or",
        5,
        "inside a numeric field's two-way binding, which is one of the \
         constructors below",
    ),
    (
        "runtime/elements.js::equal-to-unequal",
        19,
        "comparisons inside the constructors below, plus `props`'s style fold",
    ),
    (
        "runtime/elements.js::greater-than-inclusive",
        2,
        "equivalent. `props` sets `style` only for a non-empty declaration \
         set, and setting it for an empty one changes nothing: `dom.js` \
         applies a style object by iterating its entries, and an empty object \
         has none. No test can catch this one and none should have to",
    ),
    (
        "runtime/elements.js::unreached-function",
        23,
        "an element constructor no case in `elements.test.js` builds. \
         `elements.test.js` is about behaviour after construction — a binding \
         writing back, a reactive class, a filtered URL — and the *shapes* are \
         the subject of `zdc-codegen/tests/element_parity.rs`, which builds \
         every built-in through this module and compares the tree against the \
         compiler's own. Thorough, one crate away, and with constant arguments \
         only",
    ),
    (
        "runtime/elements.js::or-to-and",
        3,
        "the vector element's argument split: whether a name is one of \
         `fill`, `stroke`, `opacity` or `viewBox` — the vector's own — or one \
         of the global arguments every element takes. No case in \
         `elements.test.js` builds a vector, since the *shapes* are \
         `zdc-codegen/tests/element_parity.rs`'s subject, so reversing the \
         fold moves an argument to the other bucket with nothing here to \
         notice",
    ),
    (
        "runtime/foreign.js::unequal-to-equal",
        1,
        "`destroy` is not called when the view is cleaned up. \
         `foreign.test.js` is the contract suite — a matrix of malformed \
         imports — and the create/update/destroy ordering it deliberately \
         leaves to `zdc-codegen/tests/foreign_view.rs`, which drives it \
         through a compiled program",
    ),
    (
        "runtime/list.js::equal-to-unequal",
        2,
        "`claimRow`'s two emptiness checks, which decide whether a row can be \
         lifted out of a served document or the walk must give up and build. \
         Every list in `list.test.js` is built from nothing, so `served` is \
         `undefined` in all of them and neither check is reached with a \
         document to claim",
    ),
    (
        "runtime/list.js::less-than-inclusive",
        4,
        "two of these corrupt the longest-increasing-subsequence search that \
         decides which rows *stay put*, and the third is an off-by-one in the \
         reconciler's own `// $dev` assertion. All three still end with the \
         right list, which is what `list.test.js` asserts; what they cost is \
         moves, and how many moves a reconciliation costs is gated in \
         `crates/zdc-bench`",
    ),
    (
        "runtime/list.js::unequal-to-equal",
        1,
        "the same search, emptied outright, so every row moves. Same division \
         of labour: right list, wrong cost, and the cost is `zdc-bench`'s",
    ),
    (
        "runtime/signal.js::greater-than-inclusive",
        1,
        "GAP: the runaway-update guard fires at `STEP_LIMIT` steps instead of \
         past it. `signal.test.js` proves the guard exists by provoking a loop \
         that never settles; nothing pins the step at which it gives up, so \
         the limit could move by one — or, written the other way, by any \
         amount a provoked loop still exceeds",
    ),
    (
        "runtime/wire.js::and-to-or",
        1,
        "the wording of a `// $dev` assertion's message: the class name it \
         reports becomes the constructor itself. `wire.test.js` checks that \
         the assertion throws, not what it says",
    ),
    (
        "runtime/wire.js::equal-to-unequal",
        1,
        "the same, one message further: the label `assertEncoded` gives the \
         root of the value it is walking",
    ),
];

// --- mutating JavaScript text -------------------------------------------

/// Comments and string literals, blanked to spaces of equal length.
///
/// Offsets survive, so a position in the result is a position in the
/// original. Without this a `===` inside a comment would be mutated into a
/// mutant that cannot possibly die, and the survivor list would fill with
/// noise that hides the real ones. Borrowed in shape from
/// `check-vacuous-tests.py`, which masks for the same reason.
///
/// Regular-expression literals are not handled, because no module in
/// `MODULES` contains one — `dom-shim.js` is the only file in the
/// directory that does, and it is a test double rather than a module.
/// `a_module_that_grew_a_regex_literal_would_be_masked_wrongly` fails if
/// that changes.
/// **One byte in, one byte out.** A `—` in a comment is three bytes, and
/// blanking it to a single space would shift every offset after it — which
/// is how a mutant ends up spliced into the middle of an identifier and
/// reported as a syntax error rather than as a survivor. Multi-byte
/// sequences only ever appear inside comments and strings here, and every
/// byte of one is replaced by a space, so the result is ASCII where it is
/// blanked and byte-identical where it is not.
/// Whether a `/` at the end of `written` opens a regular expression.
///
/// The bookkeeping is here and the *decision* is `minify::starts_a_regex`,
/// which is the only place in this workspace that knows the answer.
fn starts_a_regex_here(written: &[u8]) -> bool {
    let mut k = written.len();
    while k > 0 && written[k - 1].is_ascii_whitespace() {
        k -= 1;
    }
    let previous = if k == 0 { 0 } else { written[k - 1] };
    let before = if k <= 1 { 0 } else { written[k - 2] };
    let mut start = k;
    while start > 0 && (written[start - 1].is_ascii_alphanumeric() || written[start - 1] == b'_') {
        start -= 1;
    }
    let word = std::str::from_utf8(&written[start..k]).unwrap_or_default();
    zdc_runtime::minify::starts_a_regex(previous, before, word)
}

fn masked(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let at = |k: usize| bytes.get(k).copied().unwrap_or(0);
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'/' && at(i + 1) == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
        } else if c == b'/' && at(i + 1) == b'*' {
            let mut j = i + 2;
            while j < bytes.len() && !(bytes[j] == b'*' && at(j + 1) == b'/') {
                j += 1;
            }
            let end = (j + 2).min(bytes.len());
            for byte in &bytes[i..end] {
                out.push(if *byte == b'\n' { b'\n' } else { b' ' });
            }
            i = end;
        } else if c == b'"' || c == b'\'' || c == b'`' {
            out.push(b' ');
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                let byte = bytes[i];
                out.push(if byte == b'\n' { b'\n' } else { b' ' });
                i += 1;
                if byte == c {
                    break;
                }
            }
        } else if c == b'/' && starts_a_regex_here(&out) {
            // A regex literal is text, the same as a string is: a mutant
            // placed inside `/[^\\]+/` changes the pattern rather than the
            // code, and the harness would be reporting on a character class.
            // The rule for telling one from a division is `minify`'s, shared
            // rather than copied — a second one that disagreed would be
            // wrong exactly where this is careful.
            out.push(b' ');
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                let byte = bytes[i];
                // A `[` … `]` may hold an unescaped `/`, so the class has
                // to be tracked or the literal ends early.
                out.push(if byte == b'\n' { b'\n' } else { b' ' });
                i += 1;
                if byte == b'/' {
                    break;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).expect("blanking replaces whole byte sequences with spaces")
}

/// Every top-level `function name(` in a module, as `(name, body start)`.
///
/// Top-level only — the pattern is anchored to the start of a line, with
/// `export ` optional — because a nested closure is reached exactly when
/// the function holding it is, so mutating both would double the sweep to
/// ask one question twice.
fn top_level_functions(source: &str) -> Vec<(String, usize)> {
    let masked = masked(source);
    let mut found = Vec::new();
    for (offset, line) in line_offsets(&masked) {
        let head = line.strip_prefix("export ").unwrap_or(line);
        let Some(rest) = head.strip_prefix("function ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if name.is_empty() {
            continue;
        }
        // The body opens at the first `{` after the parameter list, which
        // is the first `{` after the line's own `(` closes. A default
        // parameter holding an object literal would put a `{` earlier, so
        // the parenthesis is counted rather than assumed.
        let from = offset + (line.len() - head.len());
        let Some(open_paren) = masked[from..].find('(').map(|k| from + k) else {
            continue;
        };
        let Some(close_paren) = matching(&masked, open_paren, '(', ')') else {
            continue;
        };
        let Some(brace) = masked[close_paren..].find('{').map(|k| close_paren + k) else {
            continue;
        };
        found.push((name, brace + 1));
    }
    found
}

/// `(byte offset, line)` for every line, so a match can be spliced back
/// into the unmasked source at the same place.
fn line_offsets(source: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in source.split('\n') {
        out.push((offset, line));
        offset += line.len() + 1;
    }
    out
}

/// The index of the delimiter closing the one at `opener`.
fn matching(source: &str, opener: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (index, ch) in source.char_indices().skip_while(|(i, _)| *i < opener) {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

/// One edit to one module.
#[derive(Clone)]
struct Mutant {
    module: &'static str,
    operator: &'static str,
    occurrence: usize,
    /// What was changed, for the report a survivor produces.
    note: String,
    source: String,
}

impl Mutant {
    fn key(&self) -> String {
        format!("{}::{}", self.group(), self.occurrence)
    }

    /// The `<module>::<operator>` a survivor is counted under.
    fn group(&self) -> String {
        format!("{}::{}", self.module, self.operator)
    }
}

/// Every function in a module throws when called.
///
/// The tier-1 mutant, and the coarsest one there is: it asks only whether
/// anything runs this module. A module that survives it is not weakly
/// tested, it is untested — which is the state `runtime/list.js` was in
/// when it was missing from `MODULES`, and the state the fourth defect in
/// this file's header left a whole page in.
fn poisoned(source: &str) -> String {
    let mut out = source.to_string();
    // Back to front, so an earlier insertion cannot move a later offset.
    let mut points: Vec<usize> = top_level_functions(source)
        .into_iter()
        .map(|(_, at)| at)
        .collect();
    points.sort_unstable();
    for at in points.into_iter().rev() {
        out.insert_str(at, " throw new Error('mutant: poisoned');");
    }
    out
}

/// The textual operators, as `(name, what it finds, what it writes)`.
///
/// Small on purpose. Each one turns a *decision* into the opposite
/// decision while leaving the program parseable, which is the property
/// that makes a survivor mean something: the module still loads and still
/// runs, and every case still passed. Arithmetic and literal operators
/// were left out because most of their mutants in this runtime are
/// equivalent — a reconciler that starts at `1` instead of `0` is a
/// different program, but `+ 0` in place of `+ 1` on a debug counter is
/// not — and a sweep whose survivors are mostly equivalent mutants is a
/// sweep nobody reads.
///
/// The spaces in the relational operators are what keeps `<` out of `<=`
/// and `>` out of the arrow in `=>`; the equality operators need no such
/// help because `===` is not a substring of `!==`.
const OPERATORS: &[(&str, &str, &str)] = &[
    ("equal-to-unequal", "===", "!=="),
    ("unequal-to-equal", "!==", "==="),
    ("and-to-or", " && ", " || "),
    ("or-to-and", " || ", " && "),
    ("less-than-inclusive", " < ", " <= "),
    ("greater-than-inclusive", " > ", " >= "),
];

/// Every mutant of one module: one per function, one per operator site.
fn mutants_of(module: &'static str, source: &'static str) -> Vec<Mutant> {
    let mut out = Vec::new();

    for (occurrence, (name, at)) in top_level_functions(source).into_iter().enumerate() {
        let mut mutated = source.to_string();
        mutated.insert_str(at, " throw new Error('mutant: unreached');");
        out.push(Mutant {
            module,
            operator: "unreached-function",
            occurrence,
            note: format!("`{name}` throws when called"),
            source: mutated,
        });
    }

    let masked = masked(source);
    for (operator, find, write) in OPERATORS {
        for (occurrence, at) in find_all(&masked, find).into_iter().enumerate() {
            let mut mutated = source.to_string();
            mutated.replace_range(at..at + find.len(), write);
            out.push(Mutant {
                module,
                operator,
                occurrence,
                note: format!(
                    "`{}` became `{}` on line {}",
                    find.trim(),
                    write.trim(),
                    source[..at].matches('\n').count() + 1
                ),
                source: mutated,
            });
        }
    }

    out
}

fn find_all(haystack: &str, needle: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = haystack[from..].find(needle) {
        out.push(from + at);
        from += at + needle.len();
    }
    out
}

// --- running a suite against a mutated runtime --------------------------

/// What a suite did when handed a mutant.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The mutated module, or the suite on top of it, would not evaluate.
    /// A death, and the weakest kind: it says the module is *linked*, not
    /// that anything checks what it does.
    Unloadable(String),
    /// At least one case failed, or the suite stopped running its cases.
    Caught(String),
    /// The module loaded, every case ran, and every case passed.
    Survived,
}

/// The source of one runtime module, by the path `MODULES` gives it.
fn source_of(path: &str) -> &'static str {
    zdc_runtime::MODULES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, source)| *source)
        .unwrap_or_else(|| {
            panic!(
                "`{path}` is not in `zdc_runtime::MODULES`, so this harness would \
                 have linked nothing under that name"
            )
        })
}

/// Strip ES module syntax so the modules evaluate as one script, exactly
/// as `render.rs` does it.
fn flatten(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Evaluate one suite, with at most one module replaced by a mutant, and
/// return the case lines it reported.
///
/// `Err` is "this did not get as far as reporting" — a module that will
/// not parse, a runaway loop that spent [`LOOP_BUDGET`] at load, a suite
/// that threw outside a case.
fn report_of(detector: &Detector, patch: Option<(&str, &str)>) -> Result<Vec<String>, String> {
    let mut context = Context::default();
    context
        .runtime_limits_mut()
        .set_loop_iteration_limit(LOOP_BUDGET);

    let mut sources: Vec<(String, String)> = vec![
        ("harness".to_string(), HARNESS.to_string()),
        // Development, so the `// $dev` assertions are present: a mutant
        // that only a runtime assertion catches should be recorded as
        // caught, and a release build would be a different question.
        ("mode".to_string(), "const RELEASE = false;\n".to_string()),
        (
            "dom shim".to_string(),
            include_str!("../runtime/dom-shim.js").to_string(),
        ),
    ];
    for module in detector.modules {
        let source = match patch {
            Some((path, mutated)) if path == *module => mutated,
            _ => source_of(module),
        };
        sources.push(((*module).to_string(), flatten(source)));
    }
    sources.push((detector.suite.to_string(), flatten(detector.source)));

    for (what, source) in sources {
        if let Err(error) = context.eval(Source::from_bytes(source.as_bytes())) {
            return Err(format!("{what} did not evaluate: {error}"));
        }
    }

    let report = match context.eval(Source::from_bytes(REPORT.as_bytes())) {
        Ok(value) => value
            .to_string(&mut context)
            .map_err(|e| format!("the report is not a string: {e}"))?
            .to_std_string_escaped(),
        Err(error) => return Err(format!("the report failed: {error}")),
    };
    Ok(report
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect())
}

/// Run one suite against one mutant.
///
/// `floor` is the number of cases the unmutated run reported. A suite that
/// reports fewer has stopped running some of them, which is a death and
/// not a pass — the failure mode every suite runner in this crate guards
/// against with a hard-coded minimum, computed here instead so it cannot
/// be stale.
fn run(detector: &Detector, patch: Option<(&str, &str)>, floor: usize) -> Outcome {
    let lines = match report_of(detector, patch) {
        Ok(lines) => lines,
        Err(why) => return Outcome::Unloadable(why),
    };
    if let Some(failed) = lines.iter().find(|l| l.starts_with("FAIL")) {
        return Outcome::Caught(format!("{}: {failed}", detector.suite));
    }
    if lines.len() < floor {
        return Outcome::Caught(format!(
            "{}: reported {} cases where the unmutated run reported {floor}",
            detector.suite,
            lines.len()
        ));
    }
    Outcome::Survived
}

/// How many cases each suite reports when nothing is mutated.
///
/// Also the non-vacuity check for this whole file: a suite that reports
/// nothing, or that fails before anything is mutated, means every mutant
/// below would be recorded as caught for a reason that has nothing to do
/// with the mutation. It is where a [`LOOP_BUDGET`] set too low announces
/// itself too, rather than silently turning the whole sweep green.
fn baselines() -> BTreeMap<&'static str, usize> {
    let mut out = BTreeMap::new();
    for detector in DETECTORS {
        let lines = report_of(detector, None)
            .unwrap_or_else(|why| panic!("{} could not be set up: {why}", detector.suite));
        let failures: Vec<&String> = lines.iter().filter(|l| l.starts_with("FAIL")).collect();
        assert!(
            failures.is_empty(),
            "{} fails before anything is mutated, so every mutant below would look \
             caught:\n  {}",
            detector.suite,
            failures
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        assert!(
            !lines.is_empty(),
            "{} reported no cases at all, so it did not run",
            detector.suite
        );
        out.insert(detector.suite, lines.len());
    }
    out
}

/// The suites that link a module, its own first.
///
/// The order is what keeps the sweep affordable: every mutant stops at the
/// first suite that catches it, and the suite named after the module is by
/// a distance the likeliest to.
fn detectors_for(module: &str) -> Vec<&'static Detector> {
    let own = module
        .rsplit('/')
        .next()
        .unwrap_or(module)
        .replace(".js", "");
    let mut found: Vec<&Detector> = DETECTORS
        .iter()
        .filter(|d| d.modules.contains(&module))
        .collect();
    found.sort_by_key(|d| !d.suite.starts_with(&own));
    found
}

/// Hand a mutant to every suite that links its module, stopping at the
/// first one that notices.
fn verdict(mutant: &Mutant, floors: &BTreeMap<&'static str, usize>) -> Outcome {
    let mut weakest = Outcome::Survived;
    for detector in detectors_for(mutant.module) {
        match run(
            detector,
            Some((mutant.module, &mutant.source)),
            floors[detector.suite],
        ) {
            Outcome::Caught(why) => return Outcome::Caught(why),
            Outcome::Unloadable(why) => weakest = Outcome::Unloadable(why),
            Outcome::Survived => {}
        }
    }
    weakest
}

// --- tier one: is each module run by anything? --------------------------

/// Every module a bundle ships is either run by a suite here, or named as
/// unreached with the reason.
///
/// **This is the check the fourth defect in the header would have
/// failed.** A hand-written list of modules stopped being the whole list,
/// and every test that depended on it went on passing, because a module
/// nobody links is a module nobody can see break. Poisoning is the
/// cheapest possible question — every function in the module throws — so a
/// module that survives it is not covered at all.
#[test]
fn every_module_is_reached_by_a_suite_here() {
    let floors = baselines();
    let named: BTreeSet<&str> = UNREACHED.iter().map(|(module, _)| *module).collect();

    let mut unreached = Vec::new();
    let mut reached = Vec::new();
    for (module, source) in zdc_runtime::MODULES {
        let mutant = Mutant {
            module,
            operator: "poisoned",
            occurrence: 0,
            note: "every function in the module throws".to_string(),
            source: poisoned(source),
        };
        match verdict(&mutant, &floors) {
            Outcome::Survived => unreached.push(*module),
            Outcome::Caught(why) => reached.push(format!("{module}: {why}")),
            Outcome::Unloadable(why) => reached.push(format!("{module}: {why}")),
        }
    }

    for line in &reached {
        println!("reached  {line}");
    }
    for module in &unreached {
        println!("UNREACHED {module}");
    }

    let surprises: Vec<&&str> = unreached.iter().filter(|m| !named.contains(**m)).collect();
    assert!(
        surprises.is_empty(),
        "no suite in this crate runs {surprises:?}. Poisoning every function in \
         them changed nothing anybody checks, which means a defect in them would \
         change nothing either. Either give them a suite, or add them to \
         `UNREACHED` with the reason and where their coverage does live."
    );

    let stale: Vec<&str> = named
        .iter()
        .filter(|m| !unreached.contains(m))
        .copied()
        .collect();
    assert!(
        stale.is_empty(),
        "these are listed in `UNREACHED` and a suite here now catches a mutation \
         of them: {stale:?}. Delete the entries — a hole that has been filled must \
         not go on being described as a hole."
    );

    // Non-vacuity. `MODULES` shrinking to nothing, or every module landing
    // in `UNREACHED`, would satisfy both assertions above without this.
    assert!(
        reached.len() >= 8,
        "only {} of {} modules in `MODULES` were reached by a suite here. Eight \
         is how many this crate had suites for when the harness was written, so \
         falling below it is either the harness having stopped linking or the \
         runtime having lost coverage — and both are worth stopping for. Raise \
         this number when a suite is added; do not lower it without saying which \
         module stopped being covered and why.",
        reached.len(),
        zdc_runtime::MODULES.len()
    );
}

/// The masking in [`masked`] does not understand regular-expression
/// literals, and does not need to while no module contains one.
///
/// A `/…/` would be read as division, its contents left unmasked, and a
/// `===` inside a character class would be mutated into a mutant that
/// cannot die. That is a silent loss of accuracy rather than a failure,
/// which is exactly the kind of thing this file exists to refuse.
#[test]
fn a_module_that_grew_a_regex_literal_would_be_masked_wrongly() {
    let suspicious: Vec<&str> = zdc_runtime::MODULES
        .iter()
        .filter(|(_, source)| {
            let masked = masked(source);
            [".replace(/", ".match(/", ".test(/", ".split(/", ".exec(/"]
                .iter()
                .any(|shape| masked.contains(shape))
        })
        .map(|(name, _)| *name)
        .collect();
    assert!(
        suspicious.is_empty(),
        "{suspicious:?} now contain a regular-expression literal. `masked` treats \
         `/` as division, so the literal's contents are mutated as if they were \
         code. Teach it about regex literals before this list is trusted again."
    );
}

// --- tier two: the sweep ------------------------------------------------

/// Every mutation of every reachable module is caught by some suite here.
///
/// `#[ignore]`d and given a CI job of its own, on the same terms as the
/// browser tests: several hundred mutants, each a fresh `boa` context, is
/// minutes rather than seconds, and `cargo test` has to stay something a
/// contributor runs without thinking about it.
///
/// A survivor is not a bug in this file. It is a sentence of the form
/// "this decision in the runtime can be reversed and every test still
/// passes", which is either a gap in the suites or an equivalent mutant,
/// and either way is a thing somebody should have to write down.
#[test]
#[ignore = "several hundred mutants; has its own CI job, like the browser suite"]
fn no_mutation_of_the_runtime_goes_unnoticed() {
    let floors = baselines();
    let expected: BTreeMap<&str, usize> = SURVIVORS
        .iter()
        .map(|(group, count, _)| (*group, *count))
        .collect();
    assert_eq!(
        expected.len(),
        SURVIVORS.len(),
        "`SURVIVORS` names a group twice, so one of the two counts is being \
         thrown away"
    );
    let unreached: BTreeSet<&str> = UNREACHED.iter().map(|(module, _)| *module).collect();

    let population: Vec<Mutant> = zdc_runtime::MODULES
        .iter()
        .filter(|(module, _)| !unreached.contains(module))
        .flat_map(|(module, source)| mutants_of(module, source))
        .collect();

    // One `boa` context per mutant is the whole cost, and the contexts are
    // independent, so the sweep is spread across the machine's cores. The
    // *result* is not order-dependent: every mutant is judged by the same
    // suites either way, and the report is sorted before anything is
    // asserted about it, so a gate's answer does not depend on how many
    // cores the runner had.
    let next = AtomicUsize::new(0);
    let judged: Mutex<Vec<(String, String, String, Outcome)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 8);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(mutant) = population.get(index) else {
                    break;
                };
                let outcome = verdict(mutant, &floors);
                judged
                    .lock()
                    .expect("the result list is not poisoned")
                    .push((mutant.group(), mutant.key(), mutant.note.clone(), outcome));
            });
        }
    });

    let mut judged = judged.into_inner().expect("every worker finished");
    judged.sort_by(|a, b| a.1.cmp(&b.1));

    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut survivors = 0usize;
    let mut caught = 0usize;
    let mut unloadable = 0usize;
    for (group, key, note, outcome) in judged {
        match outcome {
            Outcome::Caught(_) => caught += 1,
            Outcome::Unloadable(_) => unloadable += 1,
            Outcome::Survived => {
                survivors += 1;
                *seen.entry(group).or_default() += 1;
                println!("SURVIVED {key}  ({note})");
            }
        }
    }

    let total = caught + unloadable + survivors;
    println!(
        "mutation: {caught} caught, {unloadable} refused to load, {survivors} \
         survived, across {total} mutants on {workers} thread(s)"
    );

    // Non-vacuity, in the shape `check-vacuous-tests.py` uses: a generator
    // that produced nothing would report a perfect score.
    assert!(
        total >= 200,
        "only {total} mutants were generated across {} modules, so the generator \
         stopped finding constructs rather than the runtime losing them",
        zdc_runtime::MODULES.len()
    );

    let mut wrong: Vec<String> = Vec::new();
    for (group, count) in &seen {
        match expected.get(group.as_str()) {
            None => wrong.push(format!(
                "{group}: {count} survivor(s) and no entry in `SURVIVORS`"
            )),
            Some(said) if said != count => wrong.push(format!(
                "{group}: `SURVIVORS` says {said}, this run found {count}"
            )),
            Some(_) => {}
        }
    }
    for group in expected.keys() {
        if !seen.contains_key(*group) {
            wrong.push(format!(
                "{group}: listed in `SURVIVORS` and every mutant in it now dies"
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "the set of mutations no suite notices has moved. Each survivor is a \
         decision the runtime makes that can be reversed with every test still \
         green — a finding about the tests, not about this file. The keys above \
         say which mutants they are:\n  {}",
        wrong.join("\n  ")
    );
}
