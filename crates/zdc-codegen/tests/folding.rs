//! **The two binder forms, executed.**
//!
//! `fold each n into total starting s to step` (#33) and `map each x in v
//! to e` (#103, #104) are the language's answer to the absence of
//! first-class functions: a lambda is *syntax*, so nothing is passed and
//! no value is a function. That argument is only worth something if what
//! comes out the far end computes the right answers, so nothing here
//! inspects generated source — every assertion is about a value that
//! survived parse, resolve, typecheck, emission and evaluation.
//!
//! The laws pinned here are the ones that are wrong quietly. A fold over
//! an empty list must be the seed rather than a crash, because
//! `Array.prototype.reduce` with no initial value throws on an empty
//! array and the emitter's one job is to pass the seed. Mapping the
//! identity over a container must give that container back, arm for arm,
//! because a rule that rebuilt `None` as `Some undefined` would still
//! render as though it worked.

mod support;

use support::{compile_source, context, rpc_context, run, run_settled};

/// Compile a program whose view shows one text signal, run it, and return
/// the text the page ended up with.
fn text(declarations: &str) -> String {
    let source = format!("{declarations}view\n    Text answer\n");
    let bundle = compile_source(&source);
    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    );
    let mut out = String::new();
    let mut inside = false;
    for ch in rendered.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out
}

// --- `fold each` ---------------------------------------------------------

/// **A fold over nothing is the seed.**
///
/// The law that is wrong loudly if the emitter forgets the initial value
/// and wrong quietly if it invents one. `reduce` with no seed throws
/// `TypeError: Reduce of empty array with no initial value`, so an empty
/// list is the case that decides whether the clause was emitted correctly
/// at all.
#[test]
fn a_fold_over_an_empty_list_is_the_seed() {
    assert_eq!(
        text(
            "state answer is client Text from text of (totalOf of empty)\n\n\
             function totalOf of ns\n    \
             from ns\n    \
             fold each n into total starting 7 to total + n\n"
        ),
        "7"
    );
}

/// **A pipeline sums a list of records by a field, in one pipeline, with
/// no helper function.**
///
/// #33's acceptance criterion, written out. Before the clause this needed
/// a second top-level function with the running total threaded through as
/// a parameter; the issue counted twenty-two of those across six example
/// programs.
#[test]
fn a_pipeline_sums_records_by_a_field_with_no_helper() {
    assert_eq!(
        text(
            "record Sale\n    amount is Whole\n\n\
             state sales is client List of Sale starting \
             [(Sale with amount is 3), (Sale with amount is 4), (Sale with amount is 5)]\n\
             state answer is client Text from text of (revenue of sales)\n\n\
             function revenue of rows\n    \
             from rows\n    \
             fold each row into total starting 0 to total + row.amount\n"
        ),
        "12"
    );
}

/// **The fold is the last clause of a pipeline the others have already
/// shaped.** #33 asked for a clause that composes with `keep`, `sort` and
/// `map each` rather than a second construct beside them.
#[test]
fn a_fold_composes_with_the_clauses_before_it() {
    assert_eq!(
        text(
            "state answer is client Text from text of (result of [1, 2, 3, 4, 5, 6])\n\n\
             function result of ns\n    \
             from ns\n    \
             keep each n where n > 2\n    \
             map each n to n * 10\n    \
             take first 3\n    \
             fold each n into total starting 0 to total + n\n"
        ),
        // 3, 4, 5, 6 survive the `keep`; 30, 40, 50, 60 the `map`; the
        // first three of those are 30 + 40 + 50.
        "120"
    );
}

/// **A fold's accumulator is not restricted to a number.**
///
/// #33's first observation was that the accumulator is almost never a
/// single scalar, and that a clause whose accumulator is one *expression*
/// handles the rest since a list, a record literal or a variant is one
/// expression. Here it is a list, built with `append`.
#[test]
fn a_fold_may_accumulate_a_list() {
    assert_eq!(
        text(
            "state answer is client Text from join with parts is (doubled of [1, 2, 3]), \
             using is \"-\"\n\n\
             function doubled of ns\n    \
             from ns\n    \
             fold each n into taken starting empty to (append (text of (n * 2)) to taken)\n"
        ),
        "2-4-6"
    );
}

/// **A record literal is a legal step, and it emits JavaScript that
/// parses.**
///
/// #33 named this as the thing to watch: the accumulator is almost never
/// one scalar, so a record in a fold's step is the *ordinary* case rather
/// than the exotic one — and an arrow body that begins with `{` is read
/// by JavaScript as a block, not an object. `js::arrow_body` is what puts
/// the parentheses there, and this is what fails if it stops.
/// `examples/sorting.zd`'s insertion sort is the same shape written out.
#[test]
fn a_fold_may_step_through_a_record_literal() {
    assert_eq!(
        text(
            "record Run\n    total is Whole\n    seen is Whole\n\n\
             state answer is client Text from shownFor of [5, 6, 7]\n\n\
             function shownFor of ns\n    \
             with run is (walk of ns)\n    \
             give (text of run.total) + \"/\" + (text of run.seen)\n\n\
             function walk of ns\n    \
             from ns\n    \
             fold each n into run starting (Run with total is 0, seen is 0) to \
             Run with total is run.total + n, seen is run.seen + 1\n"
        ),
        "18/3"
    );
}

/// **The order is left to right, and the accumulator is the left
/// operand.** A fold whose step is not commutative is the only way to see
/// which is which, and `reduce`'s argument order is the thing that would
/// silently swap them.
#[test]
fn a_fold_walks_left_to_right_with_the_total_on_the_left() {
    assert_eq!(
        text(
            "state answer is client Text from stamped of [\"a\", \"b\", \"c\"]\n\n\
             function stamped of parts\n    \
             from parts\n    \
             fold each part into joined starting \"seed:\" to joined + part\n"
        ),
        "seed:abc"
    );
}

/// The prelude's own folds, rewritten onto the clause, still answer.
///
/// `sumOf`, `countOf`, `minOf`, `maxOf` and `flatten` each lost a
/// hand-threaded helper function to this clause; these are the answers
/// that must not have moved.
#[test]
fn the_rewritten_prelude_folds_answer_as_they_did() {
    assert_eq!(
        text("state answer is client Text from text of (sumOf of [1, 2, 3])\n"),
        "6"
    );
    assert_eq!(
        text("state answer is client Text from text of (sumOf of empty)\n"),
        "0"
    );
    assert_eq!(
        text("state answer is client Text from text of (countOf of [yes, no, yes, yes])\n"),
        "3"
    );
    assert_eq!(
        text("state answer is client Text from text of (countOf of empty)\n"),
        "0"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (minOf of [3, 1, 2]), fallback is 0)\n"
        ),
        "1"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (maxOf of [3, 1, 2]), fallback is 0)\n"
        ),
        "3"
    );
    // The empty case of a fold whose seed is `None`: nothing has no
    // smallest member, and the seed is what says so.
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (minOf of empty), fallback is 0 - 1)\n"
        ),
        "-1"
    );
    assert_eq!(
        text(
            "state answer is client Text from join with parts is \
             (flatten of [[\"a\", \"b\"], empty, [\"c\"]]), using is \",\"\n"
        ),
        "a,b,c"
    );
}

// --- `map each … in …` ---------------------------------------------------

/// **`Some` is transformed and `None` is passed through.**
#[test]
fn a_payload_map_transforms_some_and_passes_none_through() {
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (map each n in ([10, 20] at 1) to n + 5), fallback is 0)\n"
        ),
        "25"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (map each n in ([10, 20] at 9) to n + 5), fallback is 0 - 1)\n"
        ),
        "-1"
    );
}

/// **Mapping the identity leaves the value alone, in both arms.**
///
/// The functor law, and the reason to pin it is that the failure is
/// invisible: a rule that rebuilt every arm as `Some` would give a `None`
/// back as `Some undefined`, and `valueOr` over that renders the string
/// `undefined` rather than the fallback — which looks like a value.
#[test]
fn mapping_the_identity_leaves_an_option_alone() {
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (map each n in ([4] at 0) to n), fallback is 0)\n"
        ),
        "4"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (isSome of \
             (map each n in ([4] at 9) to n))\n"
        ),
        "no",
        "mapping the identity over `None` produced something that is not `None`"
    );
    assert_eq!(
        text(
            "state answer is client Text from text of (isSome of \
             (map each n in ([4] at 0) to n))\n"
        ),
        "yes"
    );
}

/// **The container is evaluated once.**
///
/// The form emits two arrows and one name for exactly this: the outer
/// parameter holds the container so a call in that position runs once
/// rather than three times, and the inner one shadows it with the
/// payload. A rule that spelled the container out at each of its three
/// uses would run the call three times, which is a wrong answer the
/// moment the call is not pure.
#[test]
fn the_container_of_a_payload_map_is_evaluated_once() {
    assert_eq!(
        text(
            "state calls is client Whole starting 0\n\
             state answer is client Text from shownFor of calls\n\n\
             function shownFor of n\n    \
             give text of (valueOr with maybe is (map each v in (bumped of n) to v), fallback is 0)\n\n\
             function bumped of n\n    \
             give [n + 1] at 0\n"
        ),
        "1"
    );
}

/// **A binder inside a view is a plain value, not a getter — and each
/// instance of a component gets its own.**
///
/// Two regressions in one program, both of which compile and render.
///
/// The first is reactivity. A binder collected into the set of *reactive*
/// locals is emitted as `x()`, because that is how the runtime hands an
/// `each` binder and a `when` pattern binding over. This binder is the
/// parameter of an arrow the emitter writes and holds a plain value, so
/// calling it throws `x is not a function` at first paint. It rendered
/// `n() + 1` until `node_binders` was told to stay out of expressions.
///
/// The second is instantiation. A component's body is copied per call
/// site, and a binder that was not rebound during the copy would be shared
/// by every instance — so the second card would be typed and named against
/// the first card's payload. Three instances here, with three different
/// payloads and three different addends, so a shared binder shows up as
/// the wrong number rather than as a crash.
#[test]
fn a_payload_map_inside_a_component_is_a_value_and_is_rebound_per_instance() {
    let source = "component Card with source, bump
    Text (text of (valueOr with maybe is (map each n in source to n + bump), fallback is 0))

state a is client List of Whole starting [10]
state b is client List of Whole starting [20]

view
    Column
        Card source is (a at 0), bump is 1
        Card source is (b at 0), bump is 2
        Card source is (a at 9), bump is 5
";
    let bundle = compile_source(source);
    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    );
    for want in ["11", "22", "0"] {
        assert!(
            rendered.contains(&format!(">{want}<")),
            "expected a card showing {want}:\n{rendered}"
        );
    }
}

/// **`andThen`, which is the clause plus one five-line library
/// function.** #103 asked for `map` *and* `andThen`; the second falls out
/// of the first over `flattenOption`, with no second construct and no
/// function value anywhere.
#[test]
fn and_then_is_the_clause_over_flatten_option() {
    // `first of` gives an `Option`, so mapping it over an `Option` gives
    // an `Option of Option`, and flattening is what makes that `andThen`.
    assert_eq!(
        text(
            "state answer is client Text from text of (valueOr with maybe is \
             (flattenOption of (map each row in ([[7, 8]] at 0) to (first of row))), \
             fallback is 0)\n"
        ),
        "7"
    );
    // And the short-circuit: an inner `None` collapses to `None` rather
    // than to a `Some` wrapping one.
    assert_eq!(
        text(
            "state answer is client Text from text of (isSome of \
             (flattenOption of (map each row in ([empty] at 0) to (first of row))))\n"
        ),
        "no"
    );
}

// --- `map each … in …` over a `Remote` -----------------------------------

/// A program whose one signal is a `Remote of Text` transformed by the
/// clause. `Remote` is the type §5.2 puts the network into, so the three
/// arms are driven through the transport rather than constructed.
const OVER_A_REMOTE: &str = "\
state who is client Text starting \"Ada\"
state greeting is server Text from politeGreeting of who
state shouted is client Remote of Text from map each line in greeting to line + \"!\"

function politeGreeting of name
    give \"Hello, \" + name

view
    Column
        when shouted
            Loading show Text \"still loading\"
            Failed with e show Text \"it failed\"
            Ready with line show Text line
";

fn remote_render(transport: &str) -> String {
    let bundle = compile_source(OVER_A_REMOTE);
    let mut context = rpc_context();
    run_settled(
        &mut context,
        transport,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\n",
        "serialize($host)",
    )
}

/// **`Ready` is transformed.**
#[test]
fn a_payload_map_over_a_ready_remote_transforms_the_value() {
    let rendered = remote_render("setTransport(() => Promise.resolve('Hello, Ada'));");
    assert!(
        rendered.contains("Hello, Ada!"),
        "the clause did not transform a `Ready` payload:\n{rendered}"
    );
}

/// **`Loading` survives, which is the whole of #104.**
///
/// `readyOr` is the elimination the prelude already had and it cannot do
/// this: it collapses `Loading` and `Failed` into a fallback, which
/// `prelude/remote.zd` flags itself for. The clause keeps all three, so a
/// program can transform what came back without deciding what to do about
/// a request that has not answered.
#[test]
fn a_payload_map_over_a_loading_remote_stays_loading() {
    let rendered = remote_render("setTransport(() => new Promise(() => {}));");
    assert!(
        rendered.contains("still loading"),
        "a transform over a `Remote` that has not answered did not stay `Loading`:\n{rendered}"
    );
    assert!(
        !rendered.contains("Hello"),
        "the body ran against a payload that does not exist yet:\n{rendered}"
    );
}

/// **`Failed` survives too, carrying its error.**
#[test]
fn a_payload_map_over_a_failed_remote_stays_failed() {
    let rendered = remote_render("setTransport(() => Promise.reject(new Error('down')));");
    assert!(
        rendered.contains("it failed"),
        "a transform over a failed call did not stay `Failed`:\n{rendered}"
    );
    assert!(
        !rendered.contains("down"),
        "the host's own words reached the page:\n{rendered}"
    );
}
