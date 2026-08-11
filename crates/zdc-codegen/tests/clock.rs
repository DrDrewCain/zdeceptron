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
