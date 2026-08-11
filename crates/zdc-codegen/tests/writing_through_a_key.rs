//! `set m at k to v` on a signal this region owns — issue #253.
//!
//! The defect was an asymmetry rather than a wrong answer. Two programs
//! one word apart got opposite verdicts:
//!
//! ```zd
//! state tally is client  Map of Text to Whole starting empty   # refused
//! state tally is durable Map of Text to Whole starting empty   # accepted
//! ```
//!
//! and the refused one is the one a reader expects to be easier. A
//! `durable` write crosses to a command, and `Emitter::command` has taken
//! the path as an argument since §17.2.7 was implemented; the local write
//! hit a blanket refusal placed *before* the crossing was consulted,
//! saying the runtime had no immutable-update helper.
//!
//! That had stopped being true. `$mapSet` is exactly such a helper and
//! predates the refusal's last edit — the `insert` expression emits it.
//! So the fix is not a new helper but the same one, which is what makes
//! `set m at k to v` and `insert` agree about the resulting map by
//! construction rather than by two implementations happening to match.
//!
//! Run in the engine rather than read out of the text: the thing under
//! test is which map comes out, and a `$mapSet` call that read its
//! arguments in the wrong order would look right in the source.

mod support;

use support::{compile_source, context, refusals, run, try_compile};

const WRITES_A_KEY: &str = r#"
state tally is client Map of Text to Whole starting empty
state shown is client Text from text of (atOr with table is tally, key is "a", fallback is 0)

view
    Column
        Text shown
        Button "bump"
            on click
                set tally at "a" to 5
"#;

/// **The write lands, under the key it names.**
///
/// Asserted through a read of that key rather than through the map's
/// size, because a `$mapSet` that stored the value under the wrong key —
/// or the key under the wrong value — would leave the size right and the
/// map wrong.
#[test]
fn a_key_written_locally_holds_what_was_written() {
    let bundle = compile_source(WRITES_A_KEY);
    let mut context = context(false);
    let said = run(
        &mut context,
        &bundle.client_js,
        // `walk` and `fire`, which is how the shim drives a page: it has no
        // `querySelector` and no synthetic `click()`.
        "const $host = document.createElement('div');\n\
         main($host);\n\
         walk($host).filter((n) => n.tagName === 'button')[0].fire('click');\n\
         serialize($host)",
    );
    assert!(
        said.contains(">5<"),
        "`set tally at \"a\" to 5` did not reach the key it named: {said}"
    );
}

/// **The map it produces is the map `insert` produces.**
///
/// The two forms are one helper, and this is the assertion that says so.
/// Written as a comparison of the two results inside one program, so a
/// change to `$mapSet` cannot move one without moving the other. The
/// expression form is spelled `set k to v in m` (§14B.2), which is the
/// same three words the statement uses in a different order.
#[test]
fn writing_a_key_and_inserting_it_agree() {
    const BOTH: &str = r#"
state tally is client Map of Text to Whole starting empty
state inserted is client Map of Text to Whole from (set "a" to 5 in empty)
state written is client Whole from (atOr with table is tally, key is "a", fallback is 0)
state placed is client Whole from (atOr with table is inserted, key is "a", fallback is 0)
state same is client Truth from (written is placed)

view
    Column
        Text same
        Button "bump"
            on click
                set tally at "a" to 5
"#;
    let bundle = compile_source(BOTH);
    let mut context = context(false);
    let said = run(
        &mut context,
        &bundle.client_js,
        // `walk` and `fire`, which is how the shim drives a page: it has no
        // `querySelector` and no synthetic `click()`.
        "const $host = document.createElement('div');\n\
         main($host);\n\
         walk($host).filter((n) => n.tagName === 'button')[0].fire('click');\n\
         serialize($host)",
    );
    // `true` rather than `yes`: a `Truth` bound into the page arrives as
    // the JavaScript boolean and `bindText` prints it with JavaScript's
    // word for it. That is the language showing the host's spelling of its
    // own literal, and it is not this change's to fix — filed separately.
    assert!(
        said.contains(">true<"),
        "a written key and an inserted one produced different maps: {said}"
    );
}

/// **Placement no longer decides whether the program compiles.**
///
/// The whole of #253 in one assertion: the same statement over the same
/// type, with only the placement changed, is accepted both ways.
#[test]
fn the_same_write_is_accepted_at_both_placements() {
    for placement in ["client", "durable"] {
        let source = format!(
            "state tally is {placement} Map of Text to Whole starting empty\n\
             \n\
             view\n\
             \x20   Column\n\
             \x20       Button \"bump\"\n\
             \x20           on click\n\
             \x20               set tally at \"a\" to 5\n"
        );
        let refused = try_compile(&source, "test.zd").err();
        assert!(
            refused.is_none(),
            "a `{placement}` map refused a write through a key: {:?}",
            refused.map(|errors| errors.into_iter().map(|e| e.message).collect::<Vec<_>>())
        );
    }
}

/// **What is still refused is refused for a stated reason**, and each
/// reason is different, so one message covering all of them would be
/// telling three different readers the same unhelpful thing.
///
/// The `add` case is the one worth having a test for. The first draft of
/// `through_a_path` did not look at the operator at all, so
/// `add 1 to tally at "a"` compiled and emitted
/// `setTally($mapSet(tally(), 'a', 1))` — a **set to 1**, silently, where
/// the program said add. That is the failure this file exists to prevent
/// recurring, and it is the one a size or shape assertion would miss.
#[test]
fn the_paths_that_are_not_a_single_map_key_say_which_they_are() {
    let cases = [
        (
            "add 1 to tally at \"a\"",
            "state tally is client Map of Text to Whole starting empty",
            "Only `set` can write through a key",
        ),
        (
            "set xs at 0 to 9",
            "state xs is client List of Whole starting [1, 2, 3]",
            "writing through a key is a map operation",
        ),
    ];

    for (statement, declaration, expected) in cases {
        let source = format!(
            "{declaration}\n\
             \n\
             view\n\
             \x20   Column\n\
             \x20       Button \"bump\"\n\
             \x20           on click\n\
             \x20               {statement}\n"
        );
        let found = refusals(&source);
        assert!(
            found.iter().any(|message| message.contains(expected)),
            "`{statement}` should be refused with {expected:?}, and said: {found:?}"
        );
    }
}
