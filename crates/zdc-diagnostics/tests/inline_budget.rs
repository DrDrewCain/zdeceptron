//! The inline diagnostic stays inside its budget.
//!
//! Barik et al. (ICSE 2017, n = 56) measured that reading a compiler error
//! is about as hard as reading source code, that reading difficulty
//! significantly predicts task time, and that participants spent 13–25% of
//! their fixations on error messages. Message length costs something
//! measurable, so it is budgeted here rather than left to taste.
//!
//! **The budget.** An inline diagnostic may use:
//!
//! * one message of at most [`INLINE_MESSAGE_BUDGET`] characters — the
//!   claim, and nothing else;
//! * its spans, which are not budgeted, because §7.3 requires an
//!   information-flow rejection to *show the path* and the two-span form is
//!   the compiler's best output;
//! * one help line, which is always `run 'zdc explain <CODE>' for the
//!   rule` and nothing else.
//!
//! Everything explaining *why* lives behind `zdc explain`, where it costs
//! the reader who does not want it nothing at all.
//!
//! **Why it is a corpus and not a table of strings.** The messages are
//! built at run time from names in the program, so the only honest way to
//! measure one is to provoke it. Every fixture below is a program that
//! makes the compiler emit the code it is filed under, and the coverage
//! test at the bottom asserts that the corpus reaches every code that the
//! grammar can currently reach.

use std::collections::BTreeSet;

use zdc_diagnostics::{explain, Diagnostic, INLINE_MESSAGE_BUDGET};
use zdc_graph::GraphError;

/// Every finding — error *and* warning — that the placement and flow
/// passes report for one program.
fn findings(src: &str) -> Vec<GraphError> {
    let program = zdc_parser::parse(src)
        .unwrap_or_else(|e| panic!("fixture does not parse: {}\n{src}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| {
            let joined: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
            panic!("fixture does not resolve: {}\n{src}", joined.join("; "))
        });
    let split = zdc_graph::split(&hir);
    let mut out = split.diagnostics.clone();
    // The flow pass runs on a program the split accepted, which is the
    // same order `zdc check` uses.
    if !split.has_errors() {
        out.extend(zdc_graph::ifc(&hir, &split).diagnostics.iter().cloned());
    }
    out
}

/// One program per code, each provoking the code it is named for. Several
/// provoke more than one, which is fine: the budget is asserted over
/// everything a fixture emits, and coverage is a union.
const CORPUS: &[(&str, &str)] = &[
    (
        "E0301",
        "\
state seed  is client  Whole starting 7
state quota is durable Whole starting seed

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0311",
        "\
state hits is server Whole starting 0

view
    Column
        Button \"go\"
            on click
                add 1 to hits
",
    ),
    (
        "E0312",
        "\
state seen   is client Whole starting 0
state bumped is server Whole from bump with seen

function bump with n
    set seen to n
    give n

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0313",
        "\
secret state token is client Text starting \"\"

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0314",
        "\
state total is client Whole starting 0

function bump with box
    add 1 to box
    give 0

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0310",
        "\
state title is static Text starting \"a\"

view
    Column
        Button \"rename\"
            on click
                set title to \"b\"
",
    ),
    (
        "E0315",
        "\
state count is static Whole starting 3 emitting \"count.txt\"

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0316",
        "\
state feed is static Text starting \"x\" emitting \"/etc/hosts\"

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0320",
        "\
state a is client Whole from idOf with b
state b is client Whole from idOf with a

function idOf with n
    give n

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0321",
        "\
state base  is durable Whole starting 1
state twice is durable Whole from double with base

function double with n
    give n

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0360",
        "\
state key is client Text from environment \"K\"

view
    Column
        Text key
",
    ),
    (
        "E0361",
        "\
state page is client Text from build read \"content/hello.md\"

view
    Column
        Text page
",
    ),
    (
        "W0330",
        "\
state unread is server Text starting \"\"

view
    Column
        Text \"hi\"
",
    ),
    (
        "W0331",
        "\
state unread is client Text starting \"\"

view
    Column
        Text \"hi\"
",
    ),
    (
        "E-IFC-02",
        "\
secret state apiKey is server Text from environment \"K\"
state request is server Text from idOf with apiKey

function idOf with n
    give n

view
    Column
        Text \"hi\"
",
    ),
    (
        "E-IFC-03",
        "\
secret state apiKey is server Text from environment \"K\"
state auditLog is server Text starting \"\"
state note is server Text from stash with apiKey

function stash with key
    set auditLog to key
    give \"ok\"

view
    Column
        Text \"hi\"
",
    ),
    (
        "E-IFC-05",
        "\
secret state apiKey is server Text from environment \"K\"

view
    Column
        Text apiKey
",
    ),
    (
        "E-IFC-06",
        "\
secret state apiKey is server Text from environment \"K\"
state cached is client Text from idOf with apiKey

function idOf with n
    give n

view
    Column
        Text \"hi\"
",
    ),
    (
        "E-IFC-10",
        "\
secret state ledger is durable Text starting \"\"
state total is server Text from idOf with ledger

function idOf with n
    give n

view
    Column
        when total
            Loading        show Spinner
            Failed with e  show ErrorBar message is e.message
            Ready with got show Text got
",
    ),
];

/// One program per integrity code (spec §18.1), each provoking the code
/// it is named for.
///
/// A second corpus because integrity is reported by `zdc-types` as a
/// [`zdc_types::TypeError`] rather than by `zdc-graph` as a `GraphError`.
/// The two carry their code differently: a `GraphError` has a `code` field
/// and the renderer appends the `zdc explain` pointer from it, while a
/// `TypeError` writes the code into the sentence. Both are budgeted the
/// same way below, because the budget is about what a reader reads.
///
/// **Open:** threading a `code` field onto `TypeError` would let these
/// carry the pointer too, and would let one corpus cover both. That is a
/// change to a public type across three crates and is deliberately not
/// made inside a merge.
const INTEGRITY_CORPUS: &[(&str, &str)] = &[
    (
        "E-INT-01",
        "\
trusted state role is client Text starting \"guest\"

view
    Column
        Text role
",
    ),
    (
        "E-INT-02",
        "\
trusted state moderators is durable Map of Text to Truth starting empty
state typed is client Text starting \"\"

view
    Input typed
        on keydown with press
            set moderators at press.key to yes
",
    ),
    (
        "E-INT-03",
        "\
trusted state note is durable Text starting \"\"
state typed is client Text starting \"\"

view
    Input typed
        on keydown with press
            set note to press.key
",
    ),
    (
        "E-INT-04",
        "\
trusted state moderators is durable Map of Text to Truth starting empty
state wanted is client Truth starting no
state promoted is server Truth from promote with wanted

function promote with asked
    if asked
        set moderators at \"root\" to yes
    give yes

view
    Checkbox wanted
",
    ),
];

/// Every integrity message one program provokes.
fn integrity_messages(src: &str) -> Vec<String> {
    let program = zdc_parser::parse(src)
        .unwrap_or_else(|e| panic!("fixture does not parse: {}\n{src}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| {
            let joined: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
            panic!("fixture does not resolve: {}\n{src}", joined.join("; "))
        });
    let split = zdc_graph::split(&hir);
    match zdc_types::check(&hir, &split) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|error| error.message).collect(),
    }
}

/// Codes the corpus cannot reach, each with the reason.
///
/// These are not gaps in the corpus. Each one is a diagnostic whose
/// trigger has no syntax in the grammar as it stands, or which a second
/// pass reports first. They exist because §17.2.4's table has a column for
/// them and a table with a missing column is one nobody can check against
/// the spec.
///
/// The list is asserted to be **exactly** the set of codes the corpus
/// misses, so adding the missing syntax without adding a fixture fails
/// here, and so does deleting a fixture.
const UNREACHABLE: &[(&str, &str)] = &[
    (
        "E0302",
        "needs a scheduled trigger; no `every`/`inbound` declaration exists in the grammar",
    ),
    (
        "E0303",
        "needs both a trigger and `durable per visitor`; neither has syntax",
    ),
    (
        "E-IFC-01",
        "the split's E0313 fires on the same condition and suppresses it; it is a \
         deliberate cross-check between two passes rather than a separate mistake",
    ),
    (
        "E-IFC-07",
        "`Sink::BuildArtifact` is in the closed sink list, and no obligation site \
         constructs one yet",
    ),
    (
        "E-IFC-08",
        "`Sink::ResponseBody` is in the closed sink list, and no obligation site \
         constructs one yet",
    ),
    (
        "E-IFC-09",
        "`Sink::PlatformLog` is in the closed sink list, and no obligation site \
         constructs one yet",
    ),
];

#[test]
fn every_diagnostic_the_compiler_emits_fits_the_inline_budget() {
    let mut checked = 0;
    for (code, src) in CORPUS {
        for finding in findings(src) {
            let diagnostic = Diagnostic::from(finding.clone());
            checked += 1;

            assert_eq!(
                diagnostic.code,
                Some(finding.code),
                "a coded finding must carry its code onto the diagnostic"
            );
            assert_eq!(
                diagnostic.help.as_deref(),
                Some(explain::inline_help(finding.code).as_str()),
                "{}'s inline help must be the pointer and nothing else (fixture {code})",
                finding.code
            );
            assert!(
                diagnostic.message.chars().count() <= INLINE_MESSAGE_BUDGET,
                "{}'s inline message is {} characters, over the budget of \
                 {INLINE_MESSAGE_BUDGET} (fixture {code}):\n{}",
                finding.code,
                diagnostic.message.chars().count(),
                diagnostic.message
            );
            assert!(
                !diagnostic.message.contains('\n'),
                "{}'s inline message runs to a second paragraph (fixture {code})",
                finding.code
            );
        }
    }

    assert!(
        checked >= CORPUS.len(),
        "the corpus produced only {checked} diagnostics, which means it stopped \
         provoking them rather than that the compiler stopped emitting them"
    );
}

/// §7.3's two-span form is the thing shortening must not blunt. The
/// message got shorter; the path did not go away.
#[test]
fn the_escape_path_survives_the_shortening() {
    let src = CORPUS
        .iter()
        .find(|(code, _)| *code == "E-IFC-05")
        .map(|(_, src)| *src)
        .expect("the view-sink fixture");

    let leak = findings(src)
        .into_iter()
        .find(|f| f.code == "E-IFC-05")
        .expect("the fixture leaks into the view");

    let diagnostic = Diagnostic::from(leak);
    assert!(
        diagnostic.notes.len() >= 2,
        "the escape must still be drawn as a path, not as one span: {:?}",
        diagnostic.notes
    );
    let path: Vec<&str> = diagnostic
        .notes
        .iter()
        .map(|(_, note)| note.as_str())
        .collect();
    assert!(
        path.iter().any(|note| note.contains("declared secret")),
        "the path must still start at the declaration: {path:?}"
    );
    assert!(
        path.iter().any(|note| note.contains("in the browser")),
        "the path must still end where the browser would see it: {path:?}"
    );
    assert!(
        diagnostic.message.contains("would reach the view"),
        "the information-flow wording is unchanged: {}",
        diagnostic.message
    );
}

/// The corpus covers every code the grammar can currently reach, and the
/// list of codes it cannot reach is exactly the documented one.
#[test]
fn the_corpus_covers_every_reachable_code() {
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    for (_, src) in CORPUS {
        for finding in findings(src) {
            reached.insert(finding.code);
        }
    }

    for (_, src) in INTEGRITY_CORPUS {
        for message in integrity_messages(src) {
            for code in explain::codes() {
                if message.contains(code) {
                    reached.insert(code);
                }
            }
        }
    }

    let known: BTreeSet<&str> = explain::codes().into_iter().collect();
    let unreachable: BTreeSet<&str> = UNREACHABLE.iter().map(|(code, _)| *code).collect();

    let missed: Vec<&&str> = known.difference(&reached).collect();
    let documented: Vec<&&str> = unreachable.iter().collect();
    assert_eq!(
        missed, documented,
        "the codes with no fixture must be exactly the codes documented as \
         unreachable; add a fixture, or say why one cannot exist"
    );

    for (code, _) in CORPUS.iter().chain(INTEGRITY_CORPUS.iter()) {
        assert!(
            reached.contains(code),
            "the fixture filed under {code} no longer provokes it"
        );
    }
}

/// The integrity pass reads the same budget: a code with an unreadable
/// message is not explained by having an entry behind it.
#[test]
fn every_integrity_diagnostic_fits_the_inline_budget() {
    let mut checked = 0;
    for (code, src) in INTEGRITY_CORPUS {
        for message in integrity_messages(src) {
            checked += 1;
            assert!(
                message.chars().count() <= INLINE_MESSAGE_BUDGET,
                "an integrity message is {} characters, over the budget of \
                 {INLINE_MESSAGE_BUDGET} (fixture {code}):\n{message}",
                message.chars().count()
            );
            assert!(
                !message.contains('\n'),
                "an integrity message runs to a second paragraph (fixture {code})"
            );
        }
    }
    assert!(
        checked >= INTEGRITY_CORPUS.len(),
        "every integrity fixture must provoke at least one diagnostic"
    );
}
