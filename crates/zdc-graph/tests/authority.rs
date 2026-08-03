//! The interprocedural argument-authority fixpoint, and the sites on it.
//!
//! What is tested here is that the **fixpoint terminates and is
//! interprocedural** — a function summary is a property of a body, not of
//! its call sites, and recursion is not a special case.
//!
//! Nothing here tests that a program is free of laundering. Nothing in
//! this crate establishes it.

mod support;

use support::*;
use zdc_graph::authority::Solution;
use zdc_graph::integrity::{Authority, Writers};

/// The solved read-label of a signal, which is what G-SIG asks and what
/// fixpoint 1 answers.
fn solved(hir: &zdc_hir::Hir, name: &str) -> Authority {
    let writers = Writers::of(hir);
    Solution::solve(hir, &writers)
        .signal(def_named(hir, name))
        .0
}

// ---------------------------------------------------------------------
// Termination.
// ---------------------------------------------------------------------

const RECURSIVE: &str = r#"
function countDown with n
    if n is 0
        give 0
    give countDown with n is n - 1

state total is client Whole from countDown with n is 5

view
    Column
        Text total
"#;

/// A recursive function terminates, and lands on the right answer.
///
/// The lattice is two points and every transfer function here is built
/// from `⊔` and substitution alone, both monotone, so each summary moves
/// only upward and can move at most `params + 1` times. A cycle in the
/// call graph is therefore not a special case: `countDown` simply stops
/// changing. If the worklist ever loses that property this test hangs
/// rather than failing, which is the honest failure mode for a fixpoint.
#[test]
fn a_recursive_function_terminates() {
    let (hir, _) = compile(RECURSIVE);
    assert_eq!(solved(&hir, "total"), Authority::Trusted);
}

const MUTUAL: &str = r#"
function isEven with n
    if n is 0
        give yes
    give isOdd with n is n - 1

function isOdd with n
    if n is 0
        give no
    give isEven with n is n - 1

state answer is client Truth from isEven with n is 4

view
    Column
        Text "x"
"#;

/// A mutually recursive pair terminates. Same argument, two definitions.
#[test]
fn mutually_recursive_functions_terminate() {
    let (hir, _) = compile(MUTUAL);
    assert_eq!(solved(&hir, "answer"), Authority::Trusted);
}

// ---------------------------------------------------------------------
// The fixpoint is interprocedural, and that is the point of it.
// ---------------------------------------------------------------------

const LAUNDERS_THROUGH_A_HELPER: &str = r#"
foreign header is server
    from  "./request" as "header"
    takes name is Text
    gives Text

function whoIsIt with key
    give header with name is key

state caller is server Text from whoIsIt with key is "x-user"

view
    Column
        Text "x"
"#;

/// **The hole the fixpoint closes.**
///
/// `whoIsIt` is called with a string literal, so under "a function is
/// transparent — it computes from its arguments" its result was the join
/// of its arguments, which is Trusted. But its body reaches a
/// `foreign … is server` with no `gives trusted T`, which is Untrusted by
/// the replacement to §18.1 semantics 6, and the value that comes back is
/// whatever the request headers held. One indirection was enough to turn
/// an Untrusted value Trusted, and no rule in the grant set was violated
/// to do it — the rule was simply never asked about the callee's body.
#[test]
fn a_helper_does_not_launder_an_ungranted_foreign() {
    let (hir, _) = compile(LAUNDERS_THROUGH_A_HELPER);
    assert_eq!(
        solved(&hir, "caller"),
        Authority::Untrusted,
        "a function's result is what its body computes, not what its arguments were"
    );
}

const TWO_CALL_SITES: &str = r#"
state typed is client Text starting ""

function shout with word
    give word + "!"

state fromBox is server Text from shout with word is typed
state fromLiteral is server Text from shout with word is "hello"

view
    Column
        Input typed, hint is "say something"
"#;

/// **The summary is relational, and that is why there is no second
/// interleaved fixpoint.**
///
/// `shout` is called twice, once with a text box and once with a literal.
/// Had the analysis merged one authority per parameter and fed it back
/// into the result, the text box would have poisoned the literal call and
/// `fromLiteral` would read Untrusted — E-REL-08 would then fire at call
/// sites where nothing attacker-chosen flows. Recording the result as a
/// join over *parameter positions* keeps the two apart, and it is what
/// will let fixpoint 2 sit above this one instead of inside it.
#[test]
fn one_untrusted_call_site_does_not_poison_another() {
    let (hir, _) = compile(TWO_CALL_SITES);
    assert_eq!(solved(&hir, "fromBox"), Authority::Untrusted);
    assert_eq!(solved(&hir, "fromLiteral"), Authority::Trusted);
}
