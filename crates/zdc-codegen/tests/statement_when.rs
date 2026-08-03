//! **What a `when` does at run time**, asked of the running program rather
//! than of the emitted text.
//!
//! A statement `when` lowers to a `switch` on the tag (§16.3.10). A
//! `switch` whose arms do not each leave the block runs the *next* arm's
//! body as well, and no pass upstream of emission can see that: the split
//! and the flow pass both join over arms, and a join over-approximates
//! fall-through rather than contradicting it. So the only place the defect
//! is observable is in the answer the emitted program computes, which is
//! what every test here asserts.
//!
//! Deliberately not a byte comparison of the emission. The emission is
//! allowed to change — `break`, an `if`/`else` chain, and a labelled block
//! are all admissible lowerings of the same construct — and a test written
//! against one of them would have to be rewritten by any fix, which is
//! exactly the kind of test that stops catching the bug it was written for.

mod support;

use support::{compile_source, context, run};

/// Mount the module and report the text the counters ended up showing.
fn spans(module: &str, driver: &str) -> Vec<String> {
    let mut context = context(false);
    let frame = run(&mut context, module, driver);
    frame
        .split("<span>")
        .skip(1)
        .filter_map(|piece| piece.split("</span>").next())
        .map(str::to_string)
        .collect()
}

/// Click every button whose label is `label`, then serialise.
const CLICK: &str = r#"
const $host = document.createElement('div');
main($host);
const $go = walk($host).filter((n) => n.tagName === 'button')[0];
$go.fire('click');
serialize($host);
"#;

/// **The demonstration.** Three arms, each adding a different amount, and
/// none of them returns — an event handler has nothing to return. Only the
/// arm whose tag matched may run.
///
/// Before the fix this printed `111`: the `First` arm ran, fell out of its
/// case block, and ran `Second` and `Third` on the way out of the switch.
#[test]
fn a_statement_when_runs_only_the_arm_whose_tag_matched() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         \x20   Third\n\
         state step  is client Step  starting First\n\
         state tally is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Text tally\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               when step\n\
         \x20                   First\n\
         \x20                       add 1 to tally\n\
         \x20                   Second\n\
         \x20                       add 10 to tally\n\
         \x20                   Third\n\
         \x20                       add 100 to tally\n",
    );
    assert_eq!(
        spans(&bundle.client_js, CLICK),
        vec!["1".to_string()],
        "only the `First` arm may run:\n{}",
        bundle.client_js
    );
}

/// The middle arm, so that a fix which merely stops the *last* arm running
/// is not enough: `Second` must not reach `Third` either.
#[test]
fn a_middle_arm_does_not_reach_the_arms_after_it() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         \x20   Third\n\
         state step  is client Step  starting Second\n\
         state tally is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Text tally\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               when step\n\
         \x20                   First\n\
         \x20                       add 1 to tally\n\
         \x20                   Second\n\
         \x20                       add 10 to tally\n\
         \x20                   Third\n\
         \x20                       add 100 to tally\n",
    );
    assert_eq!(
        spans(&bundle.client_js, CLICK),
        vec!["10".to_string()],
        "`Second` must not fall into `Third`:\n{}",
        bundle.client_js
    );
}

/// Statements after the `when` still run. This is what rules out lowering
/// a non-returning arm to `return`: the handler has three more statements
/// to execute, and a `return` would silently drop them.
#[test]
fn statements_after_a_statement_when_still_run() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         state step  is client Step  starting First\n\
         state tally is client Whole starting 0\n\
         state after is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Text tally\n\
         \x20       Text after\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               when step\n\
         \x20                   First\n\
         \x20                       add 1 to tally\n\
         \x20                   Second\n\
         \x20                       add 10 to tally\n\
         \x20               add 7 to after\n",
    );
    assert_eq!(
        spans(&bundle.client_js, CLICK),
        vec!["1".to_string(), "7".to_string()],
        "the statement after the `when` must run exactly once:\n{}",
        bundle.client_js
    );
}

/// A `when` nested inside an `if`, so the fix cannot depend on the `when`
/// being the outermost statement of the handler.
#[test]
fn a_when_nested_under_an_if_does_not_fall_through() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         state step  is client Step  starting First\n\
         state armed is client Truth starting yes\n\
         state tally is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Text tally\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               if armed\n\
         \x20                   when step\n\
         \x20                       First\n\
         \x20                           add 1 to tally\n\
         \x20                       Second\n\
         \x20                           add 10 to tally\n",
    );
    assert_eq!(
        spans(&bundle.client_js, CLICK),
        vec!["1".to_string()],
        "{}",
        bundle.client_js
    );
}

/// Two `when`s in one handler, so that stopping the first cannot stop the
/// second: the `$w0`/`$w1` temporaries and the two switches are
/// independent.
#[test]
fn two_statement_whens_in_one_handler_each_run_one_arm() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         state a     is client Step  starting First\n\
         state b     is client Step  starting Second\n\
         state tally is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Text tally\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               when a\n\
         \x20                   First\n\
         \x20                       add 1 to tally\n\
         \x20                   Second\n\
         \x20                       add 10 to tally\n\
         \x20               when b\n\
         \x20                   First\n\
         \x20                       add 100 to tally\n\
         \x20                   Second\n\
         \x20                       add 1000 to tally\n",
    );
    assert_eq!(
        spans(&bundle.client_js, CLICK),
        vec!["1001".to_string()],
        "one arm from each `when`, and no more:\n{}",
        bundle.client_js
    );
}

/// A variant with fields. The binders are destructured out of `fields`
/// inside the case block, so a fall-through would also rebind `$w0.fields`
/// under the next arm's names — the wrong values, not merely extra work.
#[test]
fn an_arm_with_binders_does_not_leak_into_the_next_arms_binders() {
    let bundle = compile_source(
        "choice Note\n\
         \x20   Quiet with amount is Whole\n\
         \x20   Loud  with amount is Whole\n\
         state note  is client Note  starting Quiet with amount is 2\n\
         state tally is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Text tally\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               when note\n\
         \x20                   Quiet with amount\n\
         \x20                       add amount to tally\n\
         \x20                   Loud with amount\n\
         \x20                       add amount to tally\n",
    );
    assert_eq!(
        spans(&bundle.client_js, CLICK),
        vec!["2".to_string()],
        "the payload must be added once, not once per arm:\n{}",
        bundle.client_js
    );
}

// --- siblings: the other constructs that match on a tag -----------------

/// A `when` in a **function body**, where every arm ends in `give`. This
/// one was never broken — a `return` leaves the switch as surely as a
/// `break` does — and it is tested so that the fix is known not to have
/// broken it.
#[test]
fn a_function_when_whose_arms_all_return_still_returns_the_matched_arm() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         \x20   Third\n\
         state step  is client Step  starting Second\n\
         state tally is client Whole from weigh with step\n\
         function weigh with s\n\
         \x20   when s\n\
         \x20       First\n\
         \x20           give 1\n\
         \x20       Second\n\
         \x20           give 10\n\
         \x20       Third\n\
         \x20           give 100\n\
         view\n\
         \x20   Column\n\
         \x20       Text tally\n",
    );
    assert_eq!(
        spans(
            &bundle.client_js,
            "const $h = document.createElement('div'); main($h); serialize($h);"
        ),
        vec!["10".to_string()],
        "{}",
        bundle.client_js
    );
}

/// The `show` form of an arm, which emits `return <value>` rather than a
/// block. Also never broken, and tested for the same reason.
#[test]
fn a_function_when_written_with_show_arms_returns_the_matched_arm() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         \x20   Third\n\
         state step  is client Step  starting Third\n\
         state tally is client Whole from weigh with step\n\
         function weigh with s\n\
         \x20   when s\n\
         \x20       First  show 1\n\
         \x20       Second show 10\n\
         \x20       Third  show 100\n\
         view\n\
         \x20   Column\n\
         \x20       Text tally\n",
    );
    assert_eq!(
        spans(
            &bundle.client_js,
            "const $h = document.createElement('div'); main($h); serialize($h);"
        ),
        vec!["100".to_string()],
        "{}",
        bundle.client_js
    );
}

/// **Mixed arms**, which is the shape the defect actually reached in a
/// function body: a `show` arm returns, a block arm without `give` does
/// not, and the block arm therefore fell into whatever followed it.
#[test]
fn a_function_when_mixing_show_and_non_returning_block_arms_is_not_confused() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         \x20   Third\n\
         state step  is client Step  starting First\n\
         state tally is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Text tally\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               when step\n\
         \x20                   First\n\
         \x20                       add 1 to tally\n\
         \x20                   Second\n\
         \x20                       set tally to 500\n\
         \x20                   Third\n\
         \x20                       set tally to 900\n",
    );
    assert_eq!(
        spans(&bundle.client_js, CLICK),
        vec!["1".to_string()],
        "a `set` in a later arm must not overwrite the matched arm's work:\n{}",
        bundle.client_js
    );
}

/// The **node-position** `when` (§16.3.8). A different lowering entirely —
/// `whenInto` with one arrow per arm, so there is no switch and no
/// fall-through to have — but it matches on the same tag and is the
/// construct a reader would most expect to share the defect. Asserted
/// behaviourally so the claim is measured rather than argued.
#[test]
fn a_node_position_when_renders_only_the_arm_whose_tag_matched() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         \x20   Third\n\
         state step is client Step starting Second\n\
         view\n\
         \x20   Column\n\
         \x20       when step\n\
         \x20           First  show Text \"one\"\n\
         \x20           Second show Text \"ten\"\n\
         \x20           Third  show Text \"hundred\"\n",
    );
    assert_eq!(
        spans(
            &bundle.client_js,
            "const $h = document.createElement('div'); main($h); serialize($h);"
        ),
        vec!["ten".to_string()],
        "{}",
        bundle.client_js
    );
}

/// The same, driven through a tag change, because `whenInto` tears the old
/// arm down and mounts the new one and a leaked arm would show up as two.
#[test]
fn a_node_position_when_swaps_arms_rather_than_accumulating_them() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         state step is client Step starting First\n\
         view\n\
         \x20   Column\n\
         \x20       when step\n\
         \x20           First  show Text \"one\"\n\
         \x20           Second show Text \"two\"\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               set step to Second\n",
    );
    assert_eq!(
        spans(&bundle.client_js, CLICK),
        vec!["two".to_string()],
        "the first arm must be gone, not merely followed:\n{}",
        bundle.client_js
    );
}

/// The **server function bundle**, which prints its bodies through the
/// same `Statements::block` the client bundle does — so a `when` in a
/// function that a `server` signal derives from is the same switch, in a
/// file the browser never sees. Run here as a plain function, because a
/// function bundle imports nothing (§16.3.12 invariant 4) and so is the
/// one artefact that evaluates standalone.
#[test]
fn a_statement_when_in_a_server_function_bundle_runs_one_arm() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         \x20   Third\n\
         state step  is client Step  starting First\n\
         state tally is server Whole from weigh with step\n\
         function weigh with s\n\
         \x20   when s\n\
         \x20       First\n\
         \x20           give 1\n\
         \x20       Second\n\
         \x20           give 10\n\
         \x20       Third\n\
         \x20           give 100\n\
         view\n\
         \x20   Column\n\
         \x20       when tally\n\
         \x20           Loading        show Spinner\n\
         \x20           Failed with e  show ErrorBar message is e.message\n\
         \x20           Ready with got show Text got\n",
    );
    let function = bundle
        .functions
        .iter()
        .find(|f| f.name == "tally")
        .unwrap_or_else(|| panic!("no `tally` endpoint in {:?}", bundle.functions));
    let mut context = context(false);
    let answers = run(
        &mut context,
        &function.source,
        "[{ tag: 'First', fields: [] }, { tag: 'Second', fields: [] }, \
         { tag: 'Third', fields: [] }].map((s) => String(weigh(s))).join(',')",
    );
    assert_eq!(answers, "1,10,100", "{}", function.source);
}

/// The same bundle, with an arm that returns only on *some* paths — a
/// `give` under an `if`. This is the shape that makes the defect reachable
/// in a pure function, where there is no state to mutate and so every
/// other arm ends in `give`: with `First` and a false condition the arm
/// falls off its block, and before the fix the server answered `10`.
#[test]
fn a_server_function_arm_that_returns_only_sometimes_still_falls_out() {
    let bundle = compile_source(
        "choice Step\n\
         \x20   First\n\
         \x20   Second\n\
         state step  is client Step  starting First\n\
         state cut   is client Whole starting 0\n\
         state tally is server Whole from pick with step, cut\n\
         function pick with s, n\n\
         \x20   when s\n\
         \x20       First\n\
         \x20           if n > 0\n\
         \x20               give 1\n\
         \x20       Second\n\
         \x20           give 10\n\
         \x20   give 0\n\
         view\n\
         \x20   Column\n\
         \x20       when tally\n\
         \x20           Loading        show Spinner\n\
         \x20           Failed with e  show ErrorBar message is e.message\n\
         \x20           Ready with got show Text got\n",
    );
    let function = bundle
        .functions
        .iter()
        .find(|f| f.name == "tally")
        .unwrap_or_else(|| panic!("no `tally` endpoint in {:?}", bundle.functions));
    let mut context = context(false);
    let answers = run(
        &mut context,
        &function.source,
        "const $s = (t) => ({ tag: t, fields: [] });\n\
         [pick($s('First'), 0), pick($s('First'), 5), pick($s('Second'), 0)].join(',')",
    );
    assert_eq!(
        answers, "0,1,10",
        "a `First` that took no `give` must reach the `give 0`, not the `Second` arm:\n{}",
        function.source
    );
}

/// A `variant` value carrying fields, built and then matched in the same
/// handler. `variant` is a constructor call, not a dispatch, so it has no
/// arms of its own to fall through — but the object it builds is what
/// every `switch` above reads `.tag` off, so the round trip belongs here.
///
/// The payload-carrying arm is written **first**, so that a fall-through
/// would land in the payload-free one and overwrite what it just wrote.
#[test]
fn a_variant_with_fields_round_trips_through_a_statement_when() {
    let bundle = compile_source(
        "choice Note\n\
         \x20   Spoken with words is Text, level is Whole\n\
         \x20   Silent\n\
         state note  is client Note  starting Silent\n\
         state tally is client Whole starting 0\n\
         state shown is client Text  starting \"start\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text shown\n\
         \x20       Text tally\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               set note to Spoken with words is \"hi\", level is 4\n\
         \x20               when note\n\
         \x20                   Spoken with words, level\n\
         \x20                       set shown to words\n\
         \x20                       add level to tally\n\
         \x20                   Silent\n\
         \x20                       set shown to \"quiet\"\n",
    );
    assert_eq!(
        spans(&bundle.client_js, CLICK),
        vec!["hi".to_string(), "4".to_string()],
        "the `Spoken` arm must not be overwritten by the `Silent` one:\n{}",
        bundle.client_js
    );
}
