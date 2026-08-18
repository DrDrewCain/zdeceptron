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
        "E0317",
        "\
foreign newScene is client
    from \"./three.module.js\" as \"Scene\"
    gives new Handle

state scene is client Handle from newScene

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
        // A `static` clock, because `server` is no longer this rule's
        // business: `every` there is a scheduled job and `after` there is
        // the one remaining refusal, which `E0322-after` covers below.
        "E0322",
        "\
state digest is static Decimal every \"1m\"

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
        "E0362",
        "\
state dark is server Truth from media \"(prefers-color-scheme: dark)\"

view
    Column
        Text dark
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
    (
        "E-IFC-11",
        "\
secret state apiKey is server Text from environment \"KEY\"

view
    Column
        Image source is apiKey, alt is \"a\"
",
    ),
    (
        "E-IFC-13",
        "\
secret state apiKey is server Text from environment \"KEY\"

foreign hashOf is client
    from \"./hash.js\" as \"digest\"
    takes input is Text
    gives Text

state shown is server Text from hashOf with input is apiKey

view
    Column
        Text \"hello\"
",
    ),
    (
        "E-URL-01",
        "\
view
    Column
        Link \"javascript:alert(1)\"
            Text \"go\"
",
    ),
];

/// One program per integrity code (spec §18.1), each provoking the code
/// it is named for.
///
/// A second corpus because integrity was once reported by `zdc-types` as a
/// [`zdc_types::TypeError`] rather than by `zdc-graph` as a `GraphError`,
/// and the fixtures are still the shortest programs that reach the
/// integrity lattice. Both are budgeted the same way below, because the
/// budget is about what a reader reads.
///
/// The **open** question that used to be recorded here — whether to thread
/// a `code` field onto `TypeError` — was answered by #148: it carries one,
/// so a type error is counted on its code exactly as a graph finding is.
/// See [`TYPE_CORPUS`].
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
    (
        "E-INT-05",
        "\
foreign putObject is server
    from  \"./s3\" as \"put\"
    takes key is trusted Text, body is Text
    gives Text

state typed is client Text starting \"\"
state receipt is server Text from putObject with key is typed, body is \"hello\"

view
    Column
        Input typed, hint is \"object key\"
",
    ),
    (
        "E-REL-04",
        "\
state cards is server Text starting \"\"

release digitOracle with guess
    gives Whole
    limit 10 per visitor
    give cards

view
    Column
        Text \"x\"
",
    ),
    (
        "E-REL-08",
        "\
state typed is client Text starting \"\"

release judge with guess
    gives Truth
    limit 10 per visitor
    give yes

state verdict is server Truth from judge with guess is typed

view
    Column
        Input typed, hint is \"guess\"
",
    ),
    (
        "E-REL-10",
        "\
foreign queryParam is server
    from  \"zd:http\" as \"query\"
    takes key is Text
    gives Text

release digitOracle with guess
    gives Whole
    limit 10 per visitor
    give queryParam with key is guess

view
    Column
        Text \"x\"
",
    ),
    (
        "W-REL-01",
        "\
release judge with guess
    gives Text
    give guess

view
    Column
        Text \"x\"
",
    ),
];

/// One program per type code (§5.4's `E02…`), each provoking the code it
/// is named for.
///
/// A fourth corpus because a type error is a [`zdc_types::TypeError`],
/// reported by a pass that runs after the split and before code
/// generation. It used to be measured through [`INTEGRITY_CORPUS`], which
/// could count only the messages that spelled a code into their own
/// sentence; `TypeError` now carries a `code` field, so these are counted
/// on the code the way a graph finding is (#148).
///
/// Every fixture parses, resolves and splits — a type error is only
/// reached by a program the earlier passes accepted — and several provoke
/// more than one code, which is fine: coverage is a union.
const TYPE_CORPUS: &[(&str, &str)] = &[
    (
        "E0201",
        "\
state half is client Whole from 3 / 2

view
    Column
        Text half
",
    ),
    (
        "E0202",
        "\
state ok    is client Truth starting yes
state found is client Truth from ok contains \"x\"

view
    Column
        Text found
",
    ),
    (
        "E0203",
        "\
state words is client List of Text starting [\"a\"]
state grown is client List of Text from grow with xs is words

function grow with xs
    give append xs to xs

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0204",
        "\
function sizeOf with xs
    give length of xs

view
    Column
        Text \"hi\"
",
    ),
    (
        "E0210",
        "\
state count is client Whole starting 0

view
    Column
        when count
            Loading show Spinner
",
    ),
    (
        "E0211",
        "\
state name is client Option of Text starting None

view
    Column
        when name
            Some with value show Text value
",
    ),
    (
        "E0212",
        "\
state name is client Option of Text starting None

view
    Column
        when name
            Some with value, extra show Text value
            None                   show Text \"none\"
",
    ),
    (
        "E0220",
        "\
state half is client Decimal from halve with divisor is 2

function halve with n
    give n / 2

view
    Column
        Text half
",
    ),
    (
        "E0221",
        "\
record Post
    slug  is Text
    title is Text

state first is client Post starting Post with slug is \"a\"

view
    Column
        Text first.title
",
    ),
    (
        "E0222",
        "\
record Post
    slug is Text

state first is client Post starting Post

view
    Column
        Text first.slug
",
    ),
    (
        "E0223",
        "\
record Todo
    title is Text

state first is client Todo starting Todo with title is \"a\"

view
    Column
        Text first.name
",
    ),
    (
        "E0230",
        "\
state rows  is client List of Whole starting [1, 2]
state total is client Whole        from totalOf with xs is rows

function totalOf with xs
    from xs
    fold each x into sum starting 0 to sum + x
    keep each x where x > 0

view
    Column
        Text total
",
    ),
    (
        "E0240",
        "\
state count   is client Whole starting 0
state doubled is client Whole from count * 2

view
    Column
        Text doubled
        Button \"go\"
            on click
                set doubled to 10
",
    ),
    (
        "E0241",
        "\
state name  is client Text starting \"\"
state shout is client Text from name + \"!\"

view
    Column
        Input shout
",
    ),
    (
        "E0250",
        "\
state seed is client Whole starting 7
state page is static Whole from seed

view
    Column
        Text page
",
    ),
    (
        "E0260",
        "\
state age is client Whole starting 0

view
    Column
        NumberInput age
",
    ),
    (
        "E0270",
        "\
foreign newScene is client
    from  \"./three.js\" as \"Scene\"
    gives new Text

state scene is client Text starting \"\"

view
    Column
        Text scene
",
    ),
    (
        "E0271",
        "\
foreign Gauge is client
    from  \"./gauge.js\" as \"mount\"
    takes value is Decimal
    gives view

view
    Column
        Gauge value is 0.5
            Text \"inside\"
",
    ),
    (
        "E0272",
        "\
foreign store is client
    from  \"./db.js\" as \"put\"
    takes key is Text
    gives Text

view
    Column
        Button \"go\"
            on click
                do store with key is \"a\"
",
    ),
    (
        "E0280",
        "\
state count  is client Whole starting 0
state advice is client Text  from adviceFor with count is count

function adviceFor with count
    if count > 0
        give \"something waiting\"

view
    Column
        Text advice
",
    ),
    (
        "E0290",
        "\
state pressed is client Truth starting no

view
    Column
        Text pressed
        Button \"go\"
            on hover
                set pressed to yes
",
    ),
];

/// One malformed program per syntax code (§4.1's `E01…`), each provoking
/// the code it is named for.
///
/// A third corpus because a parse error stops the compiler before the
/// passes the other corpora run: it is a [`zdc_parser::ParseError`],
/// not a `GraphError` and not a `TypeError`. It is measured here anyway,
/// because the budget is about what a reader reads and these are the
/// messages a reader meets first.
///
/// The placement fixture is the one from the issue, verbatim.
const PARSE_CORPUS: &[(&str, &str)] = &[
    ("E0101", "state votes is Map of Id to Int starting empty\n"),
    ("E0102", "record Edge\n    from is Whole\n    to is Whole\n"),
    ("E0103", "view\n    Text (1 + 2\n"),
    ("E0104", "view\n    5\n"),
    // 96 levels of expression nesting, which no hand writes and a
    // generated file reaches.
    ("E0105", "state a is client Whole starting ((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((1))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))\n"),
    ("E0106", "route Page\n    Home is \"blog\"\n"),
    // The one code here that is not a malformed program. This parses as
    // far as the grammar is concerned and is refused for a reason the
    // grammar cannot state, so it is the fixture most likely to drift
    // into provoking something else — which is what the gate is for.
    ("E0107", "state hits is durable per visitor Whole starting 0\n"),
    // The word is reserved in the one slot it means anything in, so the
    // reader is told which construct is missing rather than that a `state`
    // declaration has no value (#18).
    ("E0108", "state paid is server Text inbound \"stripe/payment\"\n"),
];

/// Every integrity and declassification finding one program provokes, as
/// `(code, message)`.
///
/// The integrity direction used to be a second pass inside `zdc-types`,
/// over a default-open lattice, and the code was written into the message
/// text. It is now the closed lattice in `zdc-graph`, which carries the
/// code as a field. Both are read here, so the budget is measured on the
/// message the user sees and the coverage is counted on the code.
fn integrity_findings(src: &str) -> Vec<(&'static str, String)> {
    let program = zdc_parser::parse(src)
        .unwrap_or_else(|e| panic!("fixture does not parse: {}\n{src}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| {
            let joined: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
            panic!("fixture does not resolve: {}\n{src}", joined.join("; "))
        });
    let split = zdc_graph::split(&hir);
    let mut out: Vec<(&'static str, String)> = zdc_graph::ifc(&hir, &split)
        .diagnostics
        .into_iter()
        .map(|d| (d.code, d.message))
        .collect();
    if let Err(errors) = zdc_types::check(&hir, &split) {
        out.extend(errors.into_iter().map(|error| ("", error.message)));
    }
    out
}

/// Every type error one program provokes, as `(code, message)`.
///
/// The code comes off the `TypeError` rather than out of its sentence, so
/// a message may be reworded without the corpus losing sight of what it
/// provokes. An uncoded type error is reported as `None` and counted
/// nowhere: `zdc-types`'s `codes` module says which those are.
fn type_findings(src: &str) -> Vec<zdc_types::TypeError> {
    let program = zdc_parser::parse(src)
        .unwrap_or_else(|e| panic!("fixture does not parse: {}\n{src}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| {
            let joined: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
            panic!("fixture does not resolve: {}\n{src}", joined.join("; "))
        });
    let split = zdc_graph::split(&hir);
    zdc_types::check(&hir, &split).err().unwrap_or_default()
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
        "needs a trigger to read `durable per visitor` state. The trigger half has syntax \
         now — #18's `every` on a `server` declaration — so the reason this is unreachable \
         is the other half alone: `durable per visitor` is refused at the parser by \
         `E0107`, and a program cannot get as far as reading it from anywhere",
    ),
    (
        "E0364",
        "a document key handler is a *view node*, and the splitter walks the view from \
         `Ctx::CLIENT_VIEW` and from nowhere else, so no program can put one in a region \
         without a document. The refusal is stated over `Region` rather than over the two \
         regions to refuse — `Region::has_a_document`, whose total match makes a fourth \
         region a compile error — and `a_region_without_a_browser_may_not_hold_a_document_\
         listener` in `zdc-graph` is what tests it. It is defence in depth here, and saying \
         so is the alternative to a fixture that only pretends to reach it",
    ),
    (
        "E-IFC-01",
        "the split's E0313 fires on the same condition and suppresses it; it is a \
         deliberate cross-check between two passes rather than a separate mistake",
    ),
    (
        "E-IFC-07",
        "`discharge_signal` does construct this obligation — the reason here used to say \
         nothing did, and that stopped being true when `emitting` landed (#22). It is \
         unreachable for a different reason: no secret can arrive at a `static` signal for \
         it to fail on, because E0313 refuses `secret` on a `static` placement and E0301 \
         refuses a `static` read of anything that is not itself `static`. Both are \
         placement rules that happen to bound this sink; `zdc-graph`'s \
         `only_the_placement_rules_kept_a_secret_out_of_a_build_artefact` is what fails if \
         either is relaxed",
    ),
    (
        "E-IFC-09",
        "`Sink::PlatformLog` is in the closed sink list and no obligation site constructs \
         one, which `Sink::producer` states as a total function. It needs an \
         `every`/`inbound` trigger declaration to root a `BoundaryEdge::TriggerFail`, or a \
         logging call in a function bundle; `zdc-graph`'s \
         `the_platform_log_is_the_only_sink_without_a_producer` is what fails when either \
         arrives",
    ),
    // The two `zdc test` codes (issue #169). A fixture here is a source
    // string put through parse, resolve, split and flow; these two codes
    // are raised by *running* a compiled program, which is one pipeline
    // further on and needs the prelude, codegen and the JavaScript engine.
    // They are covered end to end instead, in
    // `zdc-cli/tests/expectations.rs`, against the process's real output.
    (
        "E-TEST-01",
        "raised by evaluating a compiled program, not by any analysis pass; covered \
         end to end in zdc-cli/tests/expectations.rs",
    ),
    (
        "E-TEST-02",
        "raised by evaluating a compiled program, and only when an expectation \
         exhausts the work budget or throws; covered in zdc-codegen/tests/claims.rs",
    ),
    (
        "E0363",
        "a `request` declaration lowers to a `client` signal, so its region is right \
         by construction; the check is the defence against a later change that let \
         one be reached from a serverless function, which is a sink the closed list \
         does not have",
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
        for (code, message) in integrity_findings(src) {
            for known in explain::codes() {
                if known == code || message.contains(known) {
                    reached.insert(known);
                }
            }
        }
    }

    // The syntax family, added when parse errors gained codes. It is
    // counted into the same union so that the gate below covers it: a
    // seventh parse code with no fixture, or a fixture that stops
    // provoking the code it is filed under, fails here.
    for (_, src) in PARSE_CORPUS {
        let error = zdc_parser::parse(src).expect_err("a parse fixture must not parse");
        reached.insert(error.code);
    }

    // The type family (§5.4's `E02…`), added when `TypeError` gained a
    // code field (#148). Counted on the code the error carries rather than
    // on what its sentence happens to spell, which is the whole point of
    // the field.
    for (_, src) in TYPE_CORPUS {
        for error in type_findings(src) {
            if let Some(code) = error.code {
                reached.insert(code);
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

    for (code, _) in CORPUS
        .iter()
        .chain(INTEGRITY_CORPUS.iter())
        .chain(PARSE_CORPUS.iter())
        .chain(TYPE_CORPUS.iter())
    {
        assert!(
            reached.contains(code),
            "the fixture filed under {code} no longer provokes it"
        );
    }
}

/// The syntax family reads the same budget, and carries the same pointer.
///
/// Parse errors were the longest messages in the compiler and the only
/// ones with no code: the placement error was a 210-character paragraph of
/// language documentation, which is over the budget the rest of the
/// compiler has been held to since the budget existed.
#[test]
fn every_parse_diagnostic_fits_the_budget_and_points_at_its_rule() {
    let mut checked = 0;
    for (code, src) in PARSE_CORPUS {
        let error = zdc_parser::parse(src).expect_err("a parse fixture must not parse");
        checked += 1;

        assert_eq!(
            error.code, *code,
            "the fixture filed under {code} now provokes {}",
            error.code
        );
        let length = error.message.chars().count();
        assert!(
            length <= INLINE_MESSAGE_BUDGET,
            "{code}'s inline message is {length} characters, over the budget of \
             {INLINE_MESSAGE_BUDGET}:\n{}",
            error.message
        );
        assert!(
            !error.message.contains('\n'),
            "{code}'s inline message runs to a second paragraph"
        );

        let diagnostic = Diagnostic::from(error);
        assert_eq!(
            diagnostic.help.as_deref(),
            Some(explain::inline_help(code).as_str()),
            "{code}'s inline help must be the pointer and nothing else"
        );
        assert!(
            diagnostic.label.is_some(),
            "{code} left its caret with nothing to say"
        );
    }
    assert_eq!(
        checked,
        PARSE_CORPUS.len(),
        "every parse fixture must have been measured"
    );
}

/// The integrity pass reads the same budget: a code with an unreadable
/// message is not explained by having an entry behind it.
#[test]
fn every_integrity_diagnostic_fits_the_inline_budget() {
    let mut checked = 0;
    for (code, src) in INTEGRITY_CORPUS {
        for (_, message) in integrity_findings(src) {
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

/// Every type diagnostic the corpus provokes carries a code a reader can
/// look up, and reaches them with the pointer to it.
///
/// `explanations.rs` already fails when a code in `zdc-types`'s source has
/// no entry behind it. This is the other half, and it is the half a reader
/// actually meets: a code the conversion drops on the floor is a code
/// nobody can see, and the rule behind it might as well not be written.
/// A code with no explanation is worse than no code, and the two tests
/// together are what make that impossible in either direction.
#[test]
fn every_type_diagnostic_points_a_reader_at_a_rule_that_exists() {
    let mut checked = 0;
    for (fixture, src) in TYPE_CORPUS {
        for error in type_findings(src) {
            let Some(code) = error.code else {
                continue;
            };
            checked += 1;

            assert!(
                explain::explain(code).is_some(),
                "the fixture filed under {fixture} produced {code}, which has no \
                 `zdc explain` entry"
            );

            // Whether the site had a repair of its own decides which help
            // line the reader gets, so it is read before the conversion
            // consumes the error.
            let site_help = error.help.clone();
            let diagnostic = Diagnostic::from(error);

            assert_eq!(diagnostic.code, Some(code), "fixture {fixture}");
            assert!(
                diagnostic.message.starts_with(&format!("[{code}] ")),
                "{code}'s code must reach the reader on the message (fixture {fixture}): {}",
                diagnostic.message
            );
            assert!(
                diagnostic.label.is_some(),
                "{code} left its caret with nothing to say (fixture {fixture})"
            );
            match site_help {
                // A repair that names something in this file stays.
                Some(help) => assert_eq!(diagnostic.help.as_deref(), Some(help.as_str())),
                // Everything else gets the pointer and nothing else.
                None => assert_eq!(
                    diagnostic.help.as_deref(),
                    Some(explain::inline_help(code).as_str()),
                    "{code} (fixture {fixture})"
                ),
            }
        }
    }
    assert!(
        checked >= TYPE_CORPUS.len(),
        "the type corpus produced only {checked} coded diagnostics, which means it \
         stopped provoking them rather than that the compiler stopped emitting them"
    );
}

/// The type messages that are over budget today, each with the reason.
///
/// The same shape as `scripts/check-message-budget.py`'s waiver list and
/// for the same reason: a code is what makes shortening a message
/// *possible*, because it gives the "why" somewhere to move to. #148 gave
/// the type errors codes and wrote their rules; taking the paragraphs back
/// out of the messages is the step after, and doing it in the same change
/// would have meant rewording the compiler's prose while the tests that
/// assert on it were being rewritten.
///
/// Asserted to be **exactly** the set that overruns, so it cannot grow
/// silently and cannot outlive its cause.
const TYPE_BUDGET_EXEMPT: &[(&str, &str)] = &[(
    "E0260",
    "the `NumberInput` binding restates why an empty field needs an \
     `Option`, which is now the second paragraph of `zdc explain E0260`",
)];

/// Every type message a fixture provokes fits the budget, or is one of the
/// documented overruns.
#[test]
fn every_type_diagnostic_fits_the_inline_budget_or_is_a_documented_overrun() {
    let exempt: BTreeSet<&str> = TYPE_BUDGET_EXEMPT.iter().map(|(code, _)| *code).collect();
    let mut over: BTreeSet<&str> = BTreeSet::new();
    let mut checked = 0;

    for (fixture, src) in TYPE_CORPUS {
        for error in type_findings(src) {
            checked += 1;
            assert!(
                !error.message.contains('\n'),
                "a type message runs to a second paragraph (fixture {fixture})"
            );
            if error.message.chars().count() <= INLINE_MESSAGE_BUDGET {
                continue;
            }
            let code = error.code.unwrap_or("<uncoded>");
            assert!(
                exempt.contains(code),
                "{code}'s inline message is {} characters, over the budget of \
                 {INLINE_MESSAGE_BUDGET} (fixture {fixture}):\n{}",
                error.message.chars().count(),
                error.message
            );
            over.insert(code);
        }
    }

    assert!(
        checked >= TYPE_CORPUS.len(),
        "the type corpus produced only {checked} diagnostics, which means it stopped \
         provoking them rather than that the compiler stopped emitting them"
    );
    assert_eq!(
        over, exempt,
        "the documented overruns must be exactly the messages that overrun; delete an \
         entry whose message now fits, because a waiver may not outlive its cause"
    );
}

/// The static gate reads the budget from this crate rather than carrying
/// its own copy of the number, and CI runs it.
///
/// Both halves are asserted here because both can rot silently. A gate
/// whose regular expression stops matching the declaration falls back to
/// no budget at all and reports nothing, and a gate that is not in
/// `ci.yml` is a file rather than a check — which is how the two most
/// recent additions to `scripts/` would each have failed had they not
/// been wired up in the same commit.
#[test]
fn the_static_message_gate_reads_this_crates_budget_and_runs_in_ci() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    let gate = std::fs::read_to_string(root.join("scripts/check-message-budget.py"))
        .expect("the message budget gate is in scripts/");
    assert!(
        gate.contains("pub const INLINE_MESSAGE_BUDGET: usize = (\\d+);"),
        "the gate must read the budget out of this crate, not restate it"
    );
    // The pattern it reads with has to match what this crate declares.
    // Asserted against the source rather than against the constant,
    // because the constant is what the gate cannot see.
    let declaration = std::fs::read_to_string(root.join("crates/zdc-diagnostics/src/explain.rs"))
        .expect("the budget is declared in explain.rs");
    assert!(
        declaration.contains(&format!(
            "pub const INLINE_MESSAGE_BUDGET: usize = {INLINE_MESSAGE_BUDGET};"
        )),
        "the declaration the gate matches on has been reworded"
    );

    let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("the CI workflow is readable");
    assert!(
        workflow.contains("scripts/check-message-budget.py"),
        "the gate must run in CI, or it is a file rather than a check"
    );
}
