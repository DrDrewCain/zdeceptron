//! What a clock declaration compiles to, and what it links (#19).
//!
//! **The emission is one `const` and no callback**, and that is the whole
//! claim under test. A timer in a host language is a function handed to a
//! scheduler; `every` and `after` emit no function at all, because the
//! scheduler's callback lives inside `runtime/clock.js` and does one thing
//! — put a number in a cell. Nothing a program writes can reach it.
//!
//! The behaviour of that module is `runtime/clock.test.js`'s subject, and
//! whether a real browser links and runs it is
//! `zdc-cli/tests/browser.rs::a_clock_signal_ticks_in_a_real_browser`'s.
//! What is here is the compiler's half: the text emitted, and the file set
//! shipped.

mod support;

use support::compile_source;

const CLOCKS: &str = "\
state elapsed is client Decimal every \"250ms\"
state motion is client Decimal every frame
state ready is client Truth after \"2s\"

view
    Column
        Text elapsed
        Text motion
        Text ready
";

/// The emitted declarations, verbatim.
///
/// Pinned as text rather than by substring so that a change to the shape —
/// a wrapper function, a dependency array, a scheduled effect — is a
/// failing test rather than an unnoticed drift back towards a callback.
#[test]
fn a_clock_declaration_emits_one_const_and_no_callback() {
    let bundle = compile_source(CLOCKS);
    assert!(
        bundle.client_js.contains(
            "const elapsed = everyMs(250);\n\
             const motion = everyFrame();\n\
             const ready = afterMs(2000);\n"
        ),
        "{}",
        bundle.client_js
    );
    // The import list is narrowed to what was reached, like every other
    // runtime module's.
    assert!(
        bundle
            .client_js
            .contains("import { afterMs, everyFrame, everyMs } from './runtime/clock.js';"),
        "{}",
        bundle.client_js
    );
    // No arrow, no `setInterval`, no `requestAnimationFrame` anywhere in
    // the program's own emission: the scheduler is `clock.js`'s business.
    for absent in ["setInterval", "setTimeout", "requestAnimationFrame"] {
        assert!(
            !bundle.client_js.contains(absent),
            "the emitted program named `{absent}`:\n{}",
            bundle.client_js
        );
    }
}

/// A whole millisecond count is written without a decimal point, so the
/// emission reads as the duration the program wrote.
#[test]
fn a_fractional_duration_survives_and_a_whole_one_stays_whole() {
    let bundle = compile_source(
        "state a is client Decimal every \"1.5s\"\n\
         state b is client Decimal every \"2m\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text a\n\
         \x20       Text b\n",
    );
    assert!(
        bundle.client_js.contains("everyMs(1500)"),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("everyMs(120000)"),
        "{}",
        bundle.client_js
    );
}

/// **`clock.js` ships only to the programs that use it.**
///
/// The size gate keeps a two-kilobyte reserve against Swift's number for
/// the null program, and the stated remedy when a runtime addition eats
/// into it is exactly this: a module linked only where it is reached.
/// `zdc-bench`'s `a_null_program_links_two_runtime_files` is the other
/// half — it stops the split from becoming a way of hiding bytes.
#[test]
fn only_a_program_with_a_clock_links_the_clock() {
    let with = compile_source(CLOCKS);
    assert!(
        with.runtime.contains("runtime/clock.js"),
        "a program with three clocks must link it: {:?}",
        with.runtime
    );
    // And nothing else came with it: `clock.js` imports `signal.js` alone,
    // which is the argument for it being its own file rather than part of
    // `dom.js`.
    let linked: Vec<&str> = with.runtime.iter().copied().collect();
    assert_eq!(
        linked,
        vec!["runtime/clock.js", "runtime/dom.js", "runtime/signal.js"]
    );

    let without = compile_source(
        "state n is client Whole starting 1\n\
         view\n\
         \x20   Column\n\
         \x20       Text n\n",
    );
    assert!(
        !without.runtime.contains("runtime/clock.js"),
        "a program with no clock must not ship one: {:?}",
        without.runtime
    );
}

/// A component instance's clock is declared **inside the instance**, which
/// is what puts it in the owned scope that disposes with the row or the
/// `when` arm. A clock hoisted to module scope would tick for the life of
/// the page and no test of the page could see it.
#[test]
fn a_component_local_clock_is_emitted_inside_the_instance() {
    let bundle = compile_source(
        "component Pulse\n\
         \x20   state beat is client Decimal every \"500ms\"\n\
         \x20   Row\n\
         \x20       Text beat\n\
         state showing is client Truth starting yes\n\
         view\n\
         \x20   Column\n\
         \x20       if showing\n\
         \x20           Pulse\n",
    );
    let inside = bundle
        .client_js
        .split("ifInto(")
        .nth(1)
        .expect("the conditional region");
    assert!(
        inside.contains("const beat = everyMs(500);"),
        "the clock must be built per instance:\n{}",
        bundle.client_js
    );
    // And not also at module scope, where it would outlive the instance.
    let before = bundle
        .client_js
        .split("export function main")
        .next()
        .expect("the module preamble");
    assert!(
        !before.contains("everyMs(500)"),
        "a component's clock was hoisted out of its instance:\n{}",
        bundle.client_js
    );
}

/// `from scroll` is a source signal the browser writes, in the same family
/// as the clock: nothing in the program may write it, it is `client` only,
/// and it is read like any other signal.
///
/// Asserted on the emission rather than by driving a scroll, because the
/// DOM shim these tests render against has no window to scroll — which is
/// also the case `viewport.js` answers with 0.
#[test]
fn scroll_reads_one_hoisted_cell_however_many_times_it_is_read() {
    let bundle = support::compile_source(
        "state travelled is client Decimal from scroll\n\
         state also is client Decimal from scroll\n\
         view\n\
         \x20   Column\n\
         \x20       Text travelled\n\
         \x20       Text also\n",
    );
    let js = &bundle.client_js;
    assert!(
        js.contains("import { scrollFraction } from './runtime/viewport.js';"),
        "the viewport module must be imported:\n{js}"
    );
    // One cell for the whole program: a second subscription would be a
    // second listener that always agreed with the first.
    assert_eq!(
        js.matches("scrollFraction()").count(),
        1,
        "one subscription, however many reads:\n{js}"
    );
    assert!(
        js.matches("$scroll()").count() >= 2,
        "every read goes through that one cell:\n{js}"
    );
}

/// A program that never asks where the reader is must not ship the
/// listener — §16.3.1, the rule `media.js` and `remembered.js` carry.
#[test]
fn a_program_that_never_scrolls_ships_no_viewport_module() {
    let bundle = support::compile_source("view\n    Text \"still\"\n");
    assert!(
        !bundle.client_js.contains("viewport.js"),
        "the viewport module must not be imported by a program that never reads it"
    );
}

/// Nothing may write it, exactly as nothing may write a clock.
#[test]
fn scroll_cannot_be_written() {
    let refusals = support::refusals(
        "state travelled is client Decimal from scroll\n\
         view\n\
         \x20   Column\n\
         \x20       Text travelled\n\
         \x20       Button \"reset\"\n\
         \x20           on click\n\
         \x20               set travelled to 0\n",
    );
    assert!(
        !refusals.is_empty(),
        "writing a derived scroll reading must be refused"
    );
}

/// **A clock that folds.** `every "90ms" starting v to <next>`.
///
/// The step reads the cell it writes, which is the one cycle the
/// dependency graph permits — the same one a `fold`'s accumulator is. It
/// is emitted as a thunk for exactly that reason: evaluating it at
/// declaration time would read the cell before it exists.
#[test]
fn a_stepping_clock_emits_a_thunk_that_reads_its_own_cell() {
    let bundle = support::compile_source(
        "state count is client Whole every \"200ms\" starting 0 to count + 1\n\
         view\n\
         \x20   Text (text of count)\n",
    );
    assert!(
        bundle
            .client_js
            .contains("steppingMs(200, 0, () => (count() + 1))"),
        "the step must be a thunk reading the cell:\n{}",
        bundle.client_js
    );
}

/// The frame variant, for a simulation that wants the display's own rate
/// rather than a number close to it.
#[test]
fn a_stepping_frame_clock_uses_the_frame_scheduler() {
    let bundle = support::compile_source(
        "state drift is client Decimal every frame starting 0.0 to drift + 0.5\n\
         view\n\
         \x20   Text (text of drift)\n",
    );
    assert!(
        bundle.client_js.contains("steppingFrame(0,"),
        "a stepping frame clock must reach `steppingFrame`:\n{}",
        bundle.client_js
    );
}

/// **A stepping cell accepts writes, and a plain clock does not.**
///
/// This is the difference between the two constructs and the reason
/// `NotWritable::of` takes a third argument. A plain clock's value is the
/// compiler's — elapsed milliseconds — and writing over it would be
/// writing over an answer the program never computed. A stepping clock's
/// value is the program's, and a board that ticks still has to accept
/// "press r to reset".
#[test]
fn a_stepping_cell_may_be_written_by_a_handler() {
    // `compile_source` and not `refusals`: the latter is for programs
    // expected to be turned away and panics when one compiles, which is
    // the outcome this test is about.
    let bundle = support::compile_source(
        "state count is client Whole every \"200ms\" starting 0 to count + 1\n\
         view\n\
         \x20   Column\n\
         \x20       Text (text of count)\n\
         \x20       Button \"reset\"\n\
         \x20           on click\n\
         \x20               set count to 0\n",
    );
    assert!(
        bundle.client_js.contains("const [count, "),
        "a written stepping cell must be destructured into a setter:\n{}",
        bundle.client_js
    );
}

#[test]
fn a_plain_clock_cell_still_refuses_a_write() {
    let refusals = support::refusals(
        "state elapsed is client Decimal every \"200ms\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text (text of elapsed)\n\
         \x20       Button \"reset\"\n\
         \x20           on click\n\
         \x20               set elapsed to 0.0\n",
    );
    assert!(
        !refusals.is_empty(),
        "writing an elapsed-time clock must still be refused"
    );
}

/// The step's type is the cell's, checked like any other initialiser —
/// which is what the annotation is for once the clock stops fixing it.
#[test]
fn a_step_of_the_wrong_type_is_refused() {
    let refusals = support::refusals(
        "state count is client Whole every \"200ms\" starting 0 to \"soon\"\n\
         view\n\
         \x20   Text (text of count)\n",
    );
    assert!(
        !refusals.is_empty(),
        "a step giving Text for a Whole cell must be refused"
    );
}

/// **The step's callees have to survive dead-code elimination.**
///
/// A stepping clock's step is often the *only* mention of the function it
/// folds with, so a reachability walk that visited only the initialiser
/// dropped that function from the bundle — and the emitted timer threw
/// `ReferenceError` on its first tick. A compile-time analysis producing a
/// run-time error is exactly what the walk exists to prevent, and nothing
/// else in the suite would have caught it: the module compiles, links and
/// renders, and only the second tick is wrong.
#[test]
fn a_function_reached_only_from_a_step_is_still_emitted() {
    let bundle = support::compile_source(
        "function bumped of value\n\
         \x20   give value + 2\n\
         state count is client Whole every \"200ms\" starting 0 to bumped of count\n\
         view\n\
         \x20   Text (text of count)\n",
    );
    // One spelling, not either-of-two: the emitter declares a program's
    // functions as `function <name>`, and an assertion that also accepted
    // `bumped =` would keep passing if that ever changed to a binding the
    // walk never reached. What is being tested is that the name survives
    // elimination, so the test says the one form the emitter produces.
    assert!(
        bundle.client_js.contains("function bumped"),
        "a function reached only from a step must still be emitted:\n{}",
        bundle.client_js
    );
}

/// **A step is not a derivation, so it is not a cycle.**
///
/// Reading the cell it writes is the whole construct. The graph's cycle
/// check walks initialisers, where "reads" means "recomputed when that
/// changes"; a step means "read once, when the scheduler fires", and
/// counting it would report every stepping clock as a signal defined in
/// terms of itself.
#[test]
fn a_step_reading_its_own_cell_is_not_a_cycle() {
    let bundle = support::compile_source(
        "state count is client Whole every \"200ms\" starting 0 to count + 1\n\
         view\n\
         \x20   Text (text of count)\n",
    );
    assert!(
        !bundle.client_js.is_empty(),
        "a stepping clock reading itself must compile"
    );
}

/// A cycle through two *initialisers* is still a cycle, stepping clock or
/// not — the exemption is for the step alone.
#[test]
fn two_initialisers_in_terms_of_each_other_are_still_refused() {
    let refusals = support::refusals(
        "state a is client Whole from b + 1\n\
         state b is client Whole from a + 1\n\
         view\n\
         \x20   Text (text of a)\n",
    );
    assert!(
        !refusals.is_empty(),
        "a real derivation cycle must still be refused"
    );
}

/// **The document ships painted.**
///
/// §16.3.1's shell is a `<div id=app>` and a module that fills it, so the
/// first paint used to be blank — on a slow connection, visibly so. The
/// build host runs the emitted module against a shimmed DOM and puts the
/// answer in the container.
#[cfg(feature = "evaluate")]
#[test]
fn a_document_carries_its_first_paint() {
    let bundle = support::compile_source(
        "state count is client Whole starting 7\n\
         view\n\
         \x20   Column\n\
         \x20       Heading \"Total\"\n\
         \x20       Text (text of count)\n",
    );
    let html = bundle.index_html.expect("a document");
    assert!(
        html.contains("<div id=\"app\"><div class=\"zd-col\"><h1>Total</h1>"),
        "the container must hold the rendered page:\n{html}"
    );
    // The *value*, not the template's placeholder: the prerender ran the
    // bindings, which is the difference between shipping a shape and
    // shipping a page.
    assert!(
        html.contains("<span>7</span>"),
        "bindings must have run:\n{html}"
    );
}

/// And the module adopts what it finds rather than replacing it, which is
/// what makes the painted markup worth shipping instead of thrown away.
#[test]
fn the_root_adopts_the_container_it_finds() {
    let bundle = support::compile_source(
        "state count is client Whole starting 1\n\
         view\n\
         \x20   Column\n\
         \x20       Text (text of count)\n",
    );
    assert!(
        bundle.client_js.contains(
            "if (!container.firstChild) mount($t0(), container);\n  const $r = container;"
        ),
        "the root must bind against the container it finds:\n{}",
        bundle.client_js
    );
}

/// An empty text binding is the case that breaks a naive prerender: it
/// serialises to nothing, the parser makes no text node, and the walk
/// lands on `null` before a single binding has attached. The prerendered
/// markup carries the same deliberate space the template does.
#[cfg(feature = "evaluate")]
#[test]
fn an_empty_text_binding_keeps_a_node_to_bind_to() {
    let bundle = support::compile_source(
        "state label is client Text starting \"\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text label\n",
    );
    let html = bundle.index_html.expect("a document");
    assert!(
        html.contains("<span> </span>"),
        "an empty binding must leave a text node in the markup:\n{html}"
    );
}

/// **A repeated region inside a `Scene` reads its source rather than
/// calling it.**
///
/// Every ordinary region hands the source to a runtime helper, and
/// `eachInto` unwraps a getter and takes a plain value alike — so a
/// `static` list works there without anyone thinking about it. A draw
/// list is built inside a thunk that has to produce the *array*, and the
/// first version appended `()` to every source. A reactive list was fine
/// and a `static` one reached the browser as `[…] is not a function`,
/// which is a run-time failure from a compile-time decision and visible
/// only inside a `Scene`.
#[test]
fn a_static_list_inside_a_scene_is_read_not_called() {
    // The value comes from the build host, so the harness has to supply
    // one: a `static` with no computed value is refused before emission.
    let bundle = support::try_compile_with_statics(
        "state kinds is static List of Whole starting [1, 2, 3]\n\
         view\n\
         \x20   Scene viewBox is \"0 0 60 20\"\n\
         \x20       each kind in kinds\n\
         \x20           Circle x is (kind * 15), y is 10, radius is 5\n",
        "test.zd",
        [("kinds".to_string(), "[1,2,3]".to_string())]
            .into_iter()
            .collect(),
    )
    .expect("compiles");
    assert!(
        bundle.client_js.contains("...([1,2,3]).flatMap("),
        "a static list must be spread, not called:\n{}",
        bundle.client_js
    );
}

/// The same for a conditional's condition, which had the same bug for the
/// same reason.
#[test]
fn a_static_condition_inside_a_scene_is_read_not_called() {
    let bundle = support::try_compile_with_statics(
        "state lit is static Truth starting yes\n\
         view\n\
         \x20   Scene viewBox is \"0 0 60 20\"\n\
         \x20       if lit\n\
         \x20           Segment fromX is 0, fromY is 1, toX is 6, toY is 1\n",
        "test.zd",
        [("lit".to_string(), "true".to_string())]
            .into_iter()
            .collect(),
    )
    .expect("compiles");
    assert!(
        bundle.client_js.contains("...((true) ? ["),
        "a static condition must be read, not called:\n{}",
        bundle.client_js
    );
}

/// And a *reactive* source still is called, which is the half that
/// already worked and must keep working.
#[test]
fn a_client_list_inside_a_scene_is_still_called() {
    let bundle = support::compile_source(
        "state kinds is client List of Whole starting [1, 2, 3]\n\
         view\n\
         \x20   Scene viewBox is \"0 0 60 20\"\n\
         \x20       each kind in kinds\n\
         \x20           Circle x is (kind * 15), y is 10, radius is 5\n",
    );
    // falsifiable: the two arms differ only in whether the getter needed
    // parenthesising, which is a decision `js::operand` makes from
    // precedence and which this test has no business asserting. A list read
    // *without* its getter — the defect — emits `...(kinds).flatMap(`,
    // matching neither arm.
    assert!(
        bundle.client_js.contains("...((kinds)()).flatMap(")
            || bundle.client_js.contains("...(kinds()).flatMap("),
        "a client list must still be read through its getter:\n{}",
        bundle.client_js
    );
}
