//! What the caret says, asserted on the rendered report.
//!
//! Every assertion in this file reads the text a user sees, because the
//! caret's message is a rendering decision and a test against the
//! `Diagnostic` struct alone would pass while `render` still printed
//! `here`. `zdc-diagnostics` already tests its inline budget against
//! rendered messages for the same reason.
//!
//! The two programs at the top are the ones filed in the issues: a `state`
//! declaration with no placement (#27) and a record field named with a
//! keyword (#199). They are quoted verbatim so that a change to either
//! diagnostic has to be made here as well, where the before and after are
//! both readable.

use zdc_diagnostics::{explain, render, Diagnostic, INLINE_MESSAGE_BUDGET};

/// `ariadne` colours the snippet character by character, so the plain text
/// has to be recovered before anything can be matched against it.
fn plain(rendered: &str) -> String {
    let mut out = String::new();
    let mut chars = rendered.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The report a program produces, as a user reads it.
fn report(src: &str) -> String {
    let error = zdc_parser::parse(src).expect_err("the fixture does not parse");
    plain(&render(src, "bad.zd", &Diagnostic::from(error)))
}

/// Every program in this file, so the sweeping assertions below range over
/// the same set the specific ones do.
///
/// Each one is a distinct parse rule rather than a variation on one, so
/// that "no diagnostic says only `here`" is checked across the families
/// and not against a single message four times.
const MALFORMED: &[&str] = &[
    // No placement: the canonical case of #27.
    "state votes is Map of Id to Int starting empty\n",
    // A keyword where a field name goes: the canonical case of #199.
    "record Edge\n    from is Whole\n    to is Whole\n",
    // A keyword where a state name goes.
    "state route is client Whole starting 0\n",
    // A specific token was required and something else was written.
    "view\n    Text (1 + 2\n",
    // Nothing here can begin the construct the position expects.
    "view\n    5\n",
    // A route URL that is not a canonical absolute path.
    "route Page\n    Home is \"blog\"\n",
    // A comparison chained, which has no single reading.
    "state a is client Truth starting 1 < 2 < 3\n",
];

/// #27, verbatim. The caret pointed at `Map`, a type, and said `here`; the
/// message was a paragraph of language documentation that never named the
/// word the user wrote.
#[test]
fn the_missing_placement_names_the_word_written_and_shows_the_repaired_line() {
    let out = report("state votes is Map of Id to Int starting empty\n");

    assert!(
        out.contains("`Map`"),
        "the message must name the word the user wrote:\n{out}"
    );
    assert!(
        out.contains("`Map` is the type"),
        "the caret points at a type and must say so:\n{out}"
    );
    assert!(
        out.contains("a placement goes before it"),
        "the caret must say what belongs where it is pointing:\n{out}"
    );
    assert!(
        out.contains("state votes is client Map of Id to Int starting empty"),
        "the repaired line must be shown, not described:\n{out}"
    );
    assert!(
        out.contains("zdc explain E0101"),
        "the reader must be told where the rule is:\n{out}"
    );
    assert!(
        !out.contains("browser memory"),
        "the four-placement tutorial belongs behind `zdc explain`:\n{out}"
    );
}

/// #199, verbatim. The message named the keyword and stopped.
#[test]
fn a_keyword_in_a_name_position_states_the_rule_and_the_keyword_s_own_job() {
    let out = report("record Edge\n    from is Whole\n    to is Whole\n");

    assert!(
        out.contains("`from`"),
        "the message must still name the word:\n{out}"
    );
    assert!(
        out.contains("No keyword may be a name"),
        "the message must state the rule, not only the fact:\n{out}"
    );
    assert!(
        out.contains("introduces a pipeline's source"),
        "the message must say what the keyword does elsewhere:\n{out}"
    );
    assert!(
        out.contains("zdc explain E0102"),
        "the reader must be told where the rule is:\n{out}"
    );
}

/// A different keyword in a different name position gets that keyword's
/// own role, so the sentence is built from the word rather than fixed.
#[test]
fn a_different_keyword_gets_its_own_description() {
    let out = report("state route is client Whole starting 0\n");

    assert!(
        out.contains("`route`"),
        "the message must name the word:\n{out}"
    );
    assert!(
        out.contains("names a site's URLs"),
        "`route` must be described as `route`, not as `from`:\n{out}"
    );
    assert!(
        !out.contains("pipeline"),
        "the description must not be another keyword's:\n{out}"
    );
}

/// The whole point of the change: `here` said nothing, and no diagnostic
/// may fall back to it.
#[test]
fn no_diagnostic_labels_its_caret_with_the_word_here() {
    let mut checked = 0;
    for src in MALFORMED {
        let out = report(src);
        checked += 1;
        assert!(
            !out.contains("╰─── here"),
            "this diagnostic still labels its caret `here`:\n{out}"
        );
    }
    assert_eq!(
        checked,
        MALFORMED.len(),
        "every fixture must have been rendered"
    );
}

/// A caret with nothing above a bare underline is the fallback, and it is
/// only reached when the site has nothing to add. The placement error is
/// the one that has the most to add, so it is checked directly.
#[test]
fn the_caret_carries_the_label_the_site_supplied() {
    let error = zdc_parser::parse("state votes is Map of Id to Int starting empty\n")
        .expect_err("the fixture does not parse");
    let diagnostic = Diagnostic::from(error);
    let label = diagnostic
        .label
        .as_deref()
        .expect("the placement site knows what it is pointing at");
    assert!(
        label.contains("`Map`"),
        "the label must name what the caret covers: {label}"
    );
    assert!(
        !label.contains("here"),
        "the label must say something: {label}"
    );
}

/// #27's third finding: the errors a beginner hits first were the ones
/// `zdc explain` could not answer for.
#[test]
fn every_parse_error_carries_a_code_that_zdc_explain_answers() {
    let mut checked = 0;
    for src in MALFORMED {
        let error = zdc_parser::parse(src).expect_err("the fixture does not parse");
        let code = error.code;
        checked += 1;
        assert!(
            explain::explain(code).is_some(),
            "the parse error for {src:?} carries `{code}`, which `zdc explain` \
             does not know"
        );
        let diagnostic = Diagnostic::from(error);
        assert_eq!(
            diagnostic.code,
            Some(code),
            "the code must survive the conversion to a diagnostic"
        );
    }
    assert_eq!(
        checked,
        MALFORMED.len(),
        "every fixture must have produced a coded error"
    );
}

/// The budget applied to the family it was written for but never enforced
/// on: parse errors were the longest messages in the compiler.
#[test]
fn every_parse_message_fits_the_inline_budget() {
    let mut checked = 0;
    for src in MALFORMED {
        let error = zdc_parser::parse(src).expect_err("the fixture does not parse");
        checked += 1;
        let length = error.message.chars().count();
        assert!(
            length <= INLINE_MESSAGE_BUDGET,
            "the message for {src:?} is {length} characters, over the budget of \
             {INLINE_MESSAGE_BUDGET}:\n{}",
            error.message
        );
        assert!(
            !error.message.contains('\n'),
            "the message for {src:?} runs to a second paragraph:\n{}",
            error.message
        );
    }
    assert_eq!(
        checked,
        MALFORMED.len(),
        "every fixture must have been measured"
    );
}
