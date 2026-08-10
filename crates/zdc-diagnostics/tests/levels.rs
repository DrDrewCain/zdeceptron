//! Warnings are a different thing from errors, and the difference is
//! carried rather than reconstructed.
//!
//! Before this, `Diagnostic` had no level. `zdc_graph::Severity` existed
//! and was dropped at the conversion, every report was built as
//! `ReportKind::Error`, and the CLI filtered `is_error()` before printing
//! — so `W0330` and `W0331` were computed by the split, explained by `zdc
//! explain`, and impossible for a reader to ever see. The level is what
//! makes them printable, and a [`Policy`] is what makes them arguable.

use zdc_diagnostics::{explain, render_in_colour, Diagnostic, Level, Policy, Setting};
use zdc_graph::GraphError;
use zdc_lexer::Span;

fn warning() -> Diagnostic {
    Diagnostic::from(GraphError::warning(
        "W0330",
        "nothing reads this signal",
        Span::new(6, 12),
    ))
}

fn error() -> Diagnostic {
    Diagnostic::from(GraphError::new(
        "E0301",
        "this signal cannot live where it says",
        Span::new(6, 12),
    ))
}

#[test]
fn the_producing_passs_severity_reaches_the_diagnostic() {
    assert_eq!(warning().level, Level::Warning);
    assert_eq!(error().level, Level::Error);
}

/// The level a code carries is spelled into the code. Asserted over the
/// whole published list rather than over two examples, so a new `W` code
/// cannot arrive as an error.
#[test]
fn every_published_code_agrees_with_the_level_its_letter_claims() {
    let codes = explain::codes();
    assert!(
        codes.len() >= 40,
        "the code list stopped being enumerated: {codes:?}"
    );
    let warnings: Vec<&str> = codes
        .iter()
        .copied()
        .filter(|code| Level::of(code) == Level::Warning)
        .collect();
    assert_eq!(
        warnings,
        vec!["W0330", "W0331", "W-REL-01"],
        "the warning family changed; the level and the code's letter must still agree"
    );
    for code in &codes {
        let level = Level::of(code);
        assert_eq!(
            level == Level::Warning,
            code.starts_with('W'),
            "{code} is levelled {level:?}"
        );
    }
}

/// A rendered warning says so. The heading is what a reader scanning a
/// wall of output reads first, and it used to say `Error` for both.
#[test]
fn a_warning_renders_under_a_warning_heading_and_an_error_does_not() {
    let src = "state unread is client Text starting \"\"\n";

    let warned = render_in_colour(src, "a.zd", &warning(), false);
    assert!(
        warned.contains("Warning:"),
        "a warning must not be introduced as an error:\n{warned}"
    );
    assert!(
        !warned.contains("Error:"),
        "a warning must not be introduced as an error:\n{warned}"
    );

    let failed = render_in_colour(src, "a.zd", &error(), false);
    assert!(
        failed.contains("Error:"),
        "an error is still an error:\n{failed}"
    );
}

/// The spanless path renders separately, so it is asserted separately.
#[test]
fn a_spanless_warning_is_also_introduced_as_a_warning() {
    let mut diagnostic = Diagnostic::file_error("nothing in this file is read");
    diagnostic.level = Level::Warning;

    let out = render_in_colour("", "a.zd", &diagnostic, false);

    assert!(out.starts_with("Warning: "), "{out}");
}

#[test]
fn a_policy_promotes_a_warning_to_an_error() {
    let mut diagnostic = warning();
    let policy = Policy::new().deny_warnings();

    assert!(policy.apply(&mut diagnostic));
    assert_eq!(diagnostic.level, Level::Error);
}

#[test]
fn a_policy_silences_a_warning_by_code_and_leaves_the_others_alone() {
    let policy = Policy::new().set("W0330", Setting::Silence);

    let mut silenced = warning();
    assert!(
        !policy.apply(&mut silenced),
        "the silenced code must not survive the policy"
    );

    let mut other = Diagnostic::from(GraphError::warning(
        "W0331",
        "nothing reads this signal either",
        Span::new(0, 1),
    ));
    assert!(policy.apply(&mut other));
    assert_eq!(other.level, Level::Warning);
}

/// The per-code setting is the more specific statement, so it wins.
#[test]
fn a_per_code_setting_beats_deny_warnings_in_both_directions() {
    let policy = Policy::new()
        .deny_warnings()
        .set("W0330", Setting::Warn)
        .set("W0331", Setting::Silence);

    let mut spared = warning();
    assert!(policy.apply(&mut spared));
    assert_eq!(spared.level, Level::Warning, "--warn must beat --deny");

    let mut dropped = Diagnostic::from(GraphError::warning("W0331", "unread", Span::new(0, 1)));
    assert!(
        !policy.apply(&mut dropped),
        "--allow must beat --deny for the code it names"
    );
}

/// The asymmetry is the point: a policy moves warnings and cannot move
/// errors. Asserted over every setting, because a rule with an untested
/// arm is a rule with a hole in it.
#[test]
fn no_setting_can_silence_or_demote_an_error() {
    for setting in [Setting::Silence, Setting::Warn, Setting::Deny] {
        let policy = Policy::new().deny_warnings().set("E0301", setting);
        let mut diagnostic = error();

        assert!(
            policy.apply(&mut diagnostic),
            "{setting:?} silenced an error"
        );
        assert_eq!(
            diagnostic.level,
            Level::Error,
            "{setting:?} demoted an error"
        );
    }
}

/// A diagnostic with no code cannot be named on a command line, so no
/// policy that names codes can reach it.
#[test]
fn an_uncoded_warning_is_untouched_by_a_policy_that_names_codes() {
    let mut diagnostic = Diagnostic::file_error("something worth mentioning");
    diagnostic.level = Level::Warning;
    let policy = Policy::new().set("W0330", Setting::Silence);

    assert!(policy.apply(&mut diagnostic));
    assert_eq!(diagnostic.level, Level::Warning);
}
