//! What the emitter produces, and what it refuses to produce.
//!
//! The refusals matter as much as the emissions. Every construct this
//! milestone cannot compile correctly has a test asserting it is rejected
//! with a message naming what is missing — a program that compiles to
//! something broken is worse than one that refuses.

mod support;

use support::{
    check_refusals, compile_example, compile_source, context, page, refusals, repository_path,
    resolve_refusals, run,
};

/// §16.4's worked emission for `hello.zd`, verbatim except for the heading
/// tag. §16.4 writes `<h2>`, because `Heading` was fixed at `h2`; a
/// heading's level is now its nesting depth, and this one is not nested,
/// so it is `<h1>`. That is the only difference from the worked emission,
/// and it is the whole point of the change: a document whose outline
/// starts at level two is the commonest automated accessibility failure
/// there is, and it was previously the only outline this language could
/// produce.
const HELLO: &str = r#"// zdc 0.1.1 · examples/hello.zd · generated, do not edit
import { signal } from './runtime/signal.js';
import { bindAttr, bindText, mount, on, template } from './runtime/dom.js';

const $t0 = template('<div class="zd-col"><h1>Hello, ZDeceptron</h1><input type="text" placeholder="your name"><span> </span></div>');

const [name, setName] = signal('world');

export function main(container) {
  if (!container.firstChild) mount($t0(), container);
  const $r = container;
  const $n0 = $r.firstChild;
  const $n1 = $n0.firstChild.nextSibling;
  const $n2 = $n1.nextSibling;
  bindAttr($n1, 'value', name);
  on($n1, 'input', (e) => setName(e.target.value));
  bindText($n2.firstChild, name);
  return $r;
}
"#;

/// §16.4's worked emission for `counter.zd`, verbatim except for the heading
/// tag. §16.4 writes `<h2>`, because `Heading` was fixed at `h2`; a
/// heading's level is now its nesting depth, and this one is not nested,
/// so it is `<h1>`. That is the only difference from the worked emission,
/// and it is the whole point of the change: a document whose outline
/// starts at level two is the commonest automated accessibility failure
/// there is, and it was previously the only outline this language could
/// produce.
const COUNTER: &str = r#"// zdc 0.1.1 · examples/counter.zd · generated, do not edit
import { derived, signal } from './runtime/signal.js';
import { bindText, mount, on, template } from './runtime/dom.js';

const $t0 = template('<div class="zd-col"><h1>Counter</h1><span> </span><span> </span><div class="zd-row"><button type="button">minus one</button><button type="button">plus one</button><button type="button">reset</button></div></div>');

const [count, setCount] = signal(0);
const doubled = derived(() => count() * 2);

export function main(container) {
  if (!container.firstChild) mount($t0(), container);
  const $r = container;
  const $n0 = $r.firstChild;
  const $n1 = $n0.firstChild.nextSibling;
  const $n2 = $n1.nextSibling;
  const $n3 = $n2.nextSibling.firstChild;
  const $n4 = $n3.nextSibling;
  const $n5 = $n4.nextSibling;
  bindText($n1.firstChild, count);
  bindText($n2.firstChild, doubled);
  on($n3, 'click', () => setCount(count() - 1));
  on($n4, 'click', () => setCount(count() + 1));
  on($n5, 'click', () => setCount(0));
  return $r;
}
"#;

#[test]
fn hello_matches_the_emission_the_specification_worked_out() {
    assert_eq!(compile_example("examples/hello.zd").client_js, HELLO);
}

#[test]
fn counter_matches_the_emission_the_specification_worked_out() {
    assert_eq!(compile_example("examples/counter.zd").client_js, COUNTER);
}

// --- declarations ---------------------------------------------------------

/// `HirPlace.base` is a `Res`, so whether a signal is ever written is
/// exactly decidable, and a never-written one costs no setter binding.
#[test]
fn a_never_written_signal_is_destructured_without_a_setter() {
    let client =
        compile_source("state greeting is client Text starting \"hi\"\nview\n    Text greeting\n")
            .client_js;
    assert!(
        client.contains("const [greeting] = signal('hi');"),
        "{client}"
    );
}

/// A two-way `Input` binding is a write even though no statement names it.
#[test]
fn a_signal_bound_two_way_still_gets_its_setter() {
    let client =
        compile_source("state name is client Text starting \"\"\nview\n    Input name\n").client_js;
    assert!(
        client.contains("const [name, setName] = signal('');"),
        "{client}"
    );
}

/// A signal `count` reserves `setCount`, so a program declaring one of its
/// own gets a distinct identifier rather than a duplicate declaration.
#[test]
fn a_program_name_never_collides_with_a_generated_setter() {
    let client = compile_source(
        "state count is client Whole starting 0\n\
         state setCount is client Whole starting 1\n\
         view\n\
         \x20   Column\n\
         \x20       Text count\n\
         \x20       Text setCount\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               add 1 to count\n",
    )
    .client_js;

    let declarations: Vec<&str> = client
        .lines()
        .filter(|line| line.starts_with("const "))
        .collect();
    assert_eq!(
        declarations.len(),
        3,
        "one template and two signals:\n{client}"
    );
    assert!(client.contains("setCount] = signal(0)"), "{client}");
    assert!(
        client.contains("setCount$"),
        "the program's name yields:\n{client}"
    );
}

/// The import list carries what the emission used and nothing else.
#[test]
fn the_import_list_is_narrowed_to_what_the_emission_used() {
    let client = compile_source("view\n    Heading \"static\"\n").client_js;
    assert!(
        !client.contains("signal.js"),
        "no signal is declared:\n{client}"
    );
    assert_eq!(
        client.lines().find(|line| line.starts_with("import")),
        Some("import { mount, template } from './runtime/dom.js';"),
        "{client}"
    );
}

/// Only what the client seed set reaches is emitted (§16.3.12).
#[test]
fn an_unreferenced_function_stays_out_of_the_bundle() {
    let client =
        compile_source("function unused with n\n    give n\nview\n    Heading \"hi\"\n").client_js;
    assert!(!client.contains("unused"), "{client}");
}

// --- expressions and functions -------------------------------------------

#[test]
fn a_function_reachable_from_the_view_is_emitted_and_runs() {
    let bundle = compile_source(
        "state count is client Whole starting 3\n\
         function triple with n\n\
         \x20   give n * 3\n\
         state tripled is client Whole from triple with count\n\
         view\n\
         \x20   Text tripled\n",
    );
    assert!(
        bundle
            .client_js
            .contains("function triple(n) {\n  return n * 3;\n}"),
        "{}",
        bundle.client_js
    );

    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div'); main($host); serialize($host)",
    );
    assert_eq!(rendered, "<div><span>9</span></div>");
}

/// §17.4.10's binding is a `const`, because it names one value and can
/// never take another: nothing in §14B.2's five verbs writes a local.
#[test]
fn a_local_binding_becomes_a_const_and_runs() {
    let bundle = compile_source(
        "state count is client Whole starting 3\n\
         function triple with n\n\
         \x20   with tripled is n * 3\n\
         \x20   give tripled\n\
         state shown is client Whole from triple with count\n\
         view\n\
         \x20   Text shown\n",
    );
    assert!(
        bundle
            .client_js
            .contains("function triple(n) {\n  const tripled = n * 3;\n  return tripled;\n}"),
        "{}",
        bundle.client_js
    );

    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div'); main($host); serialize($host)",
    );
    assert_eq!(rendered, "<div><span>9</span></div>");
}

/// JavaScript engines do not eliminate tail calls, so the emitter does.
/// A function that gives the result of calling itself becomes a loop, and
/// its stack depth stops depending on its input (§17.4.10).
#[test]
fn a_self_call_in_tail_position_becomes_a_loop() {
    let bundle = compile_source(
        "state n is client Whole starting 5\n\
         function countDown with left, total\n\
         \x20   if left is 0\n\
         \x20       give total\n\
         \x20   give countDown with left is left - 1, total is total + left\n\
         state answer is client Whole from countDown with left is n, total is 0\n\
         view\n\
         \x20   Text answer\n",
    );
    assert!(
        bundle.client_js.contains("$tail: while (true) {"),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("continue $tail;"),
        "{}",
        bundle.client_js
    );

    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div'); main($host); serialize($host)",
    );
    assert_eq!(rendered, "<div><span>15</span></div>");
}

/// Every argument is computed before any parameter is written, because an
/// argument is written in terms of the parameters it replaces. Swapping
/// two names is the case that catches a naive sequential assignment.
#[test]
fn a_tail_jump_computes_every_argument_before_assigning_any() {
    let bundle = compile_source(
        "state n is client Whole starting 4\n\
         function walk with a, b\n\
         \x20   if a is 0\n\
         \x20       give b\n\
         \x20   give walk with a is b - 1, b is a\n\
         state answer is client Whole from walk with a is n, b is n\n\
         view\n\
         \x20   Text answer\n",
    );
    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div'); main($host); serialize($host)",
    );
    assert_eq!(rendered, "<div><span>1</span></div>");
}

/// Only a call and nothing wrapped around it. `1 + (f with …)` still has
/// work to do when the call comes back, so it stays a call — turning it
/// into a jump would drop the addition.
#[test]
fn a_self_call_with_work_left_after_it_is_not_a_loop() {
    let bundle = compile_source(
        "state n is client Whole starting 3\n\
         function total of left\n\
         \x20   if left is 0\n\
         \x20       give 0\n\
         \x20   give left + (total of (left - 1))\n\
         state answer is client Whole from total of n\n\
         view\n\
         \x20   Text answer\n",
    );
    assert!(!bundle.client_js.contains("$tail"), "{}", bundle.client_js);

    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div'); main($host); serialize($host)",
    );
    assert_eq!(rendered, "<div><span>6</span></div>");
}

/// A call whose transitive closure touches a signal is reactive, so its
/// operand is wrapped rather than assigned once (§16.3.3).
#[test]
fn a_call_that_reads_a_signal_becomes_a_getter_and_a_call_that_does_not_does_not() {
    let reactive = compile_source(
        "state n is client Whole starting 1\n\
         function shown with x\n\
         \x20   give x * n\n\
         view\n\
         \x20   Text (shown with 2)\n",
    )
    .client_js;
    assert!(
        reactive.contains("bindText($n0.firstChild, () => shown(2));"),
        "{reactive}"
    );

    let constant = compile_source(
        "function shown with x\n\
         \x20   give x * 3\n\
         view\n\
         \x20   Text (shown with 2)\n",
    )
    .client_js;
    assert!(
        constant.contains("$n0.firstChild.nodeValue = String(shown(2));"),
        "a static value is one assignment, not an effect:\n{constant}"
    );
    assert!(!constant.contains("bindText"), "{constant}");
}

/// Parenthesising by precedence table rather than by wrapping everything.
#[test]
fn operands_are_parenthesised_only_where_the_parse_would_change() {
    let client = compile_source(
        "state a is client Whole starting 1\n\
         state b is client Whole from a * 2 - 1\n\
         state c is client Whole from a * (2 - 1)\n\
         view\n\
         \x20   Column\n\
         \x20       Text b\n\
         \x20       Text c\n",
    )
    .client_js;
    assert!(client.contains("derived(() => a() * 2 - 1)"), "{client}");
    assert!(client.contains("derived(() => a() * (2 - 1))"), "{client}");
}

// --- pipelines ------------------------------------------------------------

/// **`sort each` is stable**, and this is the test that says so.
///
/// The property is observable rather than academic: a table sorted by one
/// heading and then by another is useful if the first order survives inside
/// each group of the second, and wrong if it does not. Here the pipeline
/// sorts by `name` and then by `rank`, so a stable sort gives `cdf` (rank 1,
/// in name order) followed by `abe` (rank 2, in name order), and only a
/// stable one does.
///
/// Two assertions, because they fail for different reasons. The first pins
/// the emitted comparator: its last arm is `0`, so keys that are neither
/// less nor greater are reported equal and the elements holding them are
/// left where they were. A comparator that broke such a tie on anything
/// else would still compile and would still sort. The second runs the
/// bundle, so the guarantee is checked against a JavaScript engine
/// executing the emitted code rather than against the emitter's own idea of
/// what it wrote.
#[test]
fn a_sort_is_stable_so_a_second_sort_keeps_the_first_ones_order() {
    let bundle = compile_source(
        "record Row\n\
         \x20   rank is Whole\n\
         \x20   name is Text\n\
         function ranked of items\n\
         \x20   from items\n\
         \x20   sort each row by row.name\n\
         \x20   sort each row by row.rank\n\
         \x20   map each row to row.name\n\
         state rows is client List of Row starting [(Row with rank is 2, name is \"a\"), (Row with rank is 1, name is \"d\"), (Row with rank is 2, name is \"b\"), (Row with rank is 1, name is \"c\"), (Row with rank is 2, name is \"e\"), (Row with rank is 1, name is \"f\")]\n\
         state answer is client Text from join with parts is (ranked of rows), using is \"\"\n\
         view\n\
         \x20   Text answer\n",
    );
    assert!(
        bundle
            .client_js
            .contains("return $ka < $kb ? -1 : $ka > $kb ? 1 : 0;"),
        "the comparator must answer 0 for keys that are neither less nor greater: {}",
        bundle.client_js
    );

    let mut context = context(false);
    let rendered = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div'); main($host); serialize($host)",
    );
    assert_eq!(rendered, "<div><span>cdfabe</span></div>");
}

// --- styles ---------------------------------------------------------------

/// A static style set folds into a generated class and costs nothing at
/// runtime (§6, §16.3.11).
#[test]
fn a_static_style_becomes_a_generated_class_rather_than_an_effect() {
    let bundle = compile_source("view\n    Row padding is 8\n        Text \"a\"\n");
    assert!(
        bundle.client_js.contains(r#"<div class="zd-row zd-s0">"#),
        "{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("bindStyle"),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle.styles_css.contains(".zd-s0 { padding: 8px; }"),
        "{}",
        bundle.styles_css
    );
    assert!(
        bundle.styles_css.contains(".zd-row"),
        "the base must ship too"
    );
}

#[test]
fn a_dynamic_style_becomes_a_binding() {
    let client = compile_source(
        "state pad is client Whole starting 8\nview\n    Row padding is pad\n        Text \"a\"\n",
    )
    .client_js;
    assert!(
        client.contains("bindStyle($n0, 'padding', () => (pad)() + 'px');"),
        "{client}"
    );
}

// --- refusals -------------------------------------------------------------

fn assert_refused(source: &str, needle: &str) {
    assert_refused_by(refusals(source), source, needle);
}

/// The same assertion against codegen's own refusals — see
/// [`support::codegen_refusals`].
fn assert_refused_by_codegen(source: &str, needle: &str) {
    assert_refused_by(support::codegen_refusals(source), source, needle);
}

fn assert_refused_by(messages: Vec<String>, source: &str, needle: &str) {
    let _ = source;
    assert!(
        messages.iter().any(|message| message.contains(needle)),
        "expected a diagnostic mentioning `{needle}`, got:\n{}",
        messages.join("\n")
    );
}

#[test]
fn a_server_signal_the_browser_never_reads_costs_the_bundle_nothing() {
    // This used to be refused by name, because there was no placement
    // pass to derive a boundary from. There is now: the split reports
    // `greeting` unread (W0330), no endpoint is generated, and the client
    // bundle is the same bytes it would have been without the
    // declaration.
    let bundle =
        compile_source("state greeting is server Text starting \"hi\"\nview\n    Text \"x\"\n");
    assert!(
        !bundle.client_js.contains("greeting"),
        "{}",
        bundle.client_js
    );
    assert!(bundle.functions.is_empty());
}

#[test]
fn a_durable_write_becomes_a_command_and_a_generated_function() {
    let bundle = compile_source(
        "state visits is durable Whole starting 0\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           add 1 to visits\n",
    );
    // §16.4's line, now inside the handler's transaction: the browser
    // ships the amount, and asks once for every write it made.
    assert!(
        bundle.client_js.contains("$tx.push(['visits.incr', [1]]);"),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("await $atomic($tx);"),
        "{}",
        bundle.client_js
    );
    let function = bundle
        .functions
        .iter()
        .find(|f| f.name == "visits.incr")
        .expect("one generated command");
    assert_eq!(function.path, "functions/visits.incr.js");
    assert!(
        function.source.contains("$store.incr('visits'"),
        "{}",
        function.source
    );
}

/// §16.3.12: code generation refuses to run on a program the
/// information-flow pass rejected. It no longer refuses for want of a
/// verdict — there is one — so the refusal is now about the answer.
#[test]
fn a_secret_that_never_reaches_the_browser_compiles() {
    let bundle = compile_source(
        "secret state key is server Text from environment \"K\"\nview\n    Text \"x\"\n",
    );
    assert!(!bundle.client_js.contains("key"), "{}", bundle.client_js);
    assert!(
        !bundle.client_js.contains('K'),
        "the environment key name must not reach the browser:\n{}",
        bundle.client_js
    );
}

/// A hole is two comments in the markup and one runtime call at the walk,
/// and the arm's region is a template of its own (spec §16.3.5 P2, §16.3.8).
#[test]
fn a_view_position_when_becomes_a_hole_and_one_template_per_arm() {
    let bundle = compile_source(
        "choice Mood\n\
         \x20   Calm\n\
         \x20   Loud\n\
         state mood is client Mood starting Calm\n\
         view\n\
         \x20   Column\n\
         \x20       when mood\n\
         \x20           Calm show Text \"calm\"\n\
         \x20           Loud show Text \"loud\"\n",
    );
    let client = &bundle.client_js;
    assert!(client.contains("<!----><!---->"), "{client}");
    // Bare, never `() => mood()`: `read` unwraps exactly one level.
    assert!(
        client.contains("whenInto($n1, $n1.nextSibling, mood, {"),
        "{client}"
    );
    assert!(client.contains("'Calm': () => {"), "{client}");
    assert_eq!(client.matches("template(").count(), 3, "{client}");
}

/// A variant's binders are positional over its declared fields, and the
/// arm is written with exactly that many parameters so
/// `Function.prototype.length` is the arity `whenInto` relies on.
#[test]
fn a_when_arms_binders_are_the_variants_fields_positionally() {
    let bundle = compile_source(
        "choice Note\n\
         \x20   Silent\n\
         \x20   Spoken with words is Text, loudness is Whole\n\
         state note is client Note starting Silent\n\
         view\n\
         \x20   Column\n\
         \x20       when note\n\
         \x20           Silent show Text \"nothing\"\n\
         \x20           Spoken with what, level\n\
         \x20               Text what\n\
         \x20               Text level\n",
    );
    let client = &bundle.client_js;
    assert!(client.contains("'Spoken': (what, level) => {"), "{client}");
    assert!(
        client.contains("bindText($n2.firstChild, what)"),
        "{client}"
    );
    assert!(
        client.contains("bindText($n3.firstChild, level)"),
        "{client}"
    );
}

/// §16.3.9: the list is a getter, the key function is `$byPosition`, and
/// the row's binder is a getter because the row outlives its item.
#[test]
fn a_view_position_each_becomes_a_keyed_hole() {
    let bundle = compile_source(
        "state items is client List of Text starting [\"a\", \"b\"]\n\
         view\n\
         \x20   Column\n\
         \x20       each item in items\n\
         \x20           Text item\n",
    );
    let client = &bundle.client_js;
    assert!(
        client.contains("const $byPosition = (item, index) => index;"),
        "{client}"
    );
    assert!(
        client.contains("eachInto($n1, $n1.nextSibling, items, $byPosition, (item) => {"),
        "{client}"
    );
    assert!(
        client.contains("bindText($n2.firstChild, item)"),
        "{client}"
    );
}

/// A module with no list declares no key function.
#[test]
fn a_module_without_a_list_declares_no_key_function() {
    let bundle = compile_example("examples/counter.zd");
    assert!(!bundle.client_js.contains("$byPosition"));
}

/// §4.4 ratifies a leading text slot on `Row` and `Column`: the value is
/// one text node, and the children follow it.
#[test]
fn a_leading_argument_to_row_becomes_a_text_node_before_the_children() {
    let client = compile_source("view\n    Row \"label\"\n        Text \"a\"\n").client_js;
    assert!(
        client.contains(r#"<div class="zd-row">label<span>a</span></div>"#),
        "{client}"
    );
}

/// A bare text node is not a `<span>`, which is what keeps the leading
/// slot and a nested `Text` from being two phrasings of one thing (§4.1).
#[test]
fn a_leading_argument_and_a_nested_text_are_different_trees() {
    let leading = compile_source("view\n    Row \"a\"\n").client_js;
    let nested = compile_source("view\n    Row\n        Text \"a\"\n").client_js;
    assert!(
        leading.contains(r#"<div class="zd-row">a</div>"#),
        "{leading}"
    );
    assert!(
        nested.contains(r#"<div class="zd-row"><span>a</span></div>"#),
        "{nested}"
    );
}

/// The slot widened to `Row` and `Column` and to nothing else: an element
/// whose content is entirely nested still refuses a leading argument, and
/// the checker is the layer that says so.
#[test]
fn an_element_with_no_slot_still_refuses_a_leading_argument() {
    let refusals = check_refusals("view\n    Main \"label\"\n");
    assert!(
        refusals[0].contains("takes no leading value"),
        "{refusals:?}"
    );
}

#[test]
fn a_second_handler_for_a_two_way_binding_is_refused() {
    assert_refused(
        "state name is client Text starting \"\"\n\
         view\n\
         \x20   Input name\n\
         \x20       on input\n\
         \x20           set name to \"x\"\n",
        "already wires",
    );
}

#[test]
fn an_element_that_shows_one_value_refuses_children() {
    assert_refused(
        "view\n    Text \"a\"\n        Text \"b\"\n",
        "takes no children",
    );
}

/// §16.7 item 6: the checker says which container `empty` is, and the two
/// have different literals.
#[test]
fn empty_becomes_the_container_the_checker_named() {
    // `xs` has to be *read* for its initialiser to be emitted at all:
    // §17.2.6 replaced "every client signal is a client seed", so an
    // unread signal costs no cell and no setter (it gets W0331 from the
    // split instead). Both halves below therefore read what they declare.
    let list = compile_source(
        "state xs is client List of Text starting empty\n\
         view\n\
         \x20   each x in xs\n\
         \x20       Text x\n",
    );
    assert!(list.client_js.contains("signal([])"), "{}", list.client_js);

    let map = compile_source(
        "state xs is client Map of Text to Whole starting empty\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           remove \"a\" from xs\n",
    );
    assert!(
        map.client_js.contains("signal(new Map())"),
        "{}",
        map.client_js
    );
}

/// §16.7 item 5 was only half a type question. The checker always said
/// which container this is; what was missing was the helper that builds
/// the `Option of T` §5.4 promises — §14F's standard library, which is now
/// the prelude's primitive layer (§17.4.7).
#[test]
fn at_builds_the_option_5_4_promises() {
    let bundle = compile_source(
        "state xs is client List of Whole starting []\n\
         state one is client Option of Whole from xs at 0\n\
         view\n\
         \x20   when one\n\
         \x20       Some with value show Text value\n\
         \x20       None            show Text \"none\"\n",
    );
    assert!(
        bundle.client_js.contains("$listAt(xs(), 0)"),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("const $listAt ="),
        "the helper is inlined, never imported:\n{}",
        bundle.client_js
    );
}

/// A helper is emitted only where it is reached, so the closure claim in
/// §14A.1 covers the library exactly as it covers a program's own
/// functions.
#[test]
fn a_program_that_never_indexes_carries_no_index_helper() {
    let bundle = compile_source("state n is client Whole starting 1\nview\n    Text n\n");
    assert!(
        !bundle.client_js.contains("$listAt"),
        "{}",
        bundle.client_js
    );
}

/// §17.4.3: which of the three `contains` functions this is comes off the
/// checker's verdict, and the one it chose is emitted with it. The other
/// two are not, which is the dead-code claim applied to the library.
#[test]
fn contains_emits_the_library_function_the_checker_chose() {
    let bundle = compile_source(
        "state words is client List of Text starting []\n\
         state found is client Truth from words contains \"a\"\n\
         view\n\
         \x20   Text found\n",
    );
    assert!(
        bundle.client_js.contains("listContains(words(), 'a')"),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("function listContains("),
        "the library function it dispatched to must be in the bundle:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("function mapContains("),
        "the ones it did not dispatch to must not be:\n{}",
        bundle.client_js
    );
}

/// `length of` over each of its three containers, per §17.4.3's table.
#[test]
fn length_of_reads_the_property_each_container_has() {
    let bundle = compile_source(
        "state xs is client List of Whole starting []\n\
         state m is client Map of Text to Whole starting empty\n\
         state s is client Text starting \"\"\n\
         state a is client Whole from length of xs\n\
         state b is client Whole from length of m\n\
         state c is client Whole from length of s\n\
         view\n\
         \x20   Text a\n\
         \x20   Text b\n\
         \x20   Text c\n",
    );
    assert!(
        bundle.client_js.contains("xs().length"),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("m().size"),
        "{}",
        bundle.client_js
    );
    assert!(
        bundle.client_js.contains("$textLength(s())"),
        "{}",
        bundle.client_js
    );
}

/// `add` through a key is still refused, and the reason moved from
/// "§14B.3 has not settled" to what is actually unsettled — see #253.
///
/// `set` through a key now compiles; the verbs that read the old value
/// first do not, because a key that is absent has no old value. The
/// refusal names that rather than an unsettled section number, and
/// `writing_through_a_key.rs` holds the accepted side.
#[test]
fn adding_through_a_path_is_refused_because_there_is_nothing_to_add_to() {
    assert_refused(
        "state scores is client Map of Whole to Whole starting empty\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           add 1 to scores at 0\n",
        "Only `set` can write through a key",
    );
}

// --- modules --------------------------------------------------------------
//
// A file with no `view` is a module, not a mistake (§14D.2): it declares
// names for other files to import and renders nothing. It builds to the
// module and stops there.

/// The module's own declarations are exported, and there is no `main` and
/// no page — §16.3.1's page imports a `main` a module does not have.
#[test]
fn a_program_without_a_view_builds_to_an_importable_module() {
    let bundle = compile_example("examples/model.zd");
    assert!(
        bundle.client_js.contains("export function visible(all) {"),
        "a module's declarations are importable:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("main("),
        "a module renders nothing, so it exports no entry point:\n{}",
        bundle.client_js
    );
    assert_eq!(
        bundle.index_html, None,
        "a page importing a `main` that does not exist would throw on load"
    );
}

/// §14D.2 makes every top-level declaration importable, so the walk cannot
/// prune to the seed set an application uses — the importer is outside this
/// compilation unit. A helper only another declaration reaches is emitted.
#[test]
fn every_top_level_declaration_of_a_module_is_emitted() {
    let bundle = compile_source(
        "state count is client Whole starting 2\n\
         function twice with n\n\
         \x20   give n * 2\n\
         function quadrupled with n\n\
         \x20   give twice with (twice with n)\n",
    );
    for declaration in [
        "export function twice(n) {",
        "export function quadrupled(",
        "export const [count] = signal(2);",
    ] {
        assert!(
            bundle.client_js.contains(declaration),
            "`{declaration}` is importable and must be emitted:\n{}",
            bundle.client_js
        );
    }
}

/// Emitting is not the claim: the module has to *run*. This evaluates it
/// and calls what it exports.
#[test]
fn a_modules_exports_run_when_the_importing_code_calls_them() {
    let bundle = compile_example("examples/model.zd");
    let mut context = context(false);
    let answer = run(
        &mut context,
        &bundle.client_js,
        // `visible` is `take first 20`, so a list of 25 comes back as 20
        // and a list of 3 comes back whole.
        "const $long = Array.from({ length: 25 }, (_, i) => i);\n\
         visible($long).length + ',' + visible([1, 2, 3]).join('') + ',' + visible($long)[19]",
    );
    assert_eq!(answer, "20,123,19");
}

/// §16.7 items 1 and 2, now answered. `+` is emitted because the checker
/// has proved both operands numeric or both `Text`, and `is` is emitted
/// because it has proved the operand is a base type.
#[test]
fn addition_and_equality_emit_the_operators_the_specification_chose() {
    let source = "state a is client Whole starting 1\n\
                  state sum is client Whole from a + 1\n\
                  state same is client Truth from a is 1\n\
                  view\n\
                  \x20   Column\n\
                  \x20       Text sum\n\
                  \x20       Text same\n";
    let bundle = compile_source(source);
    assert!(
        bundle.client_js.contains("derived(() => a() + 1)"),
        "{}",
        bundle.client_js
    );
    // `===`, not `Object.is`: `-0 === 0` is true, which is the right answer
    // in an f64 language and what Elm and Dart do.
    assert!(
        bundle.client_js.contains("derived(() => a() === 1)"),
        "{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("Object.is"),
        "{}",
        bundle.client_js
    );
}

/// And still refused when the checker could not settle the operand, which
/// is what stops `1 + "a"` coercing (§5.4) and what stops `===` silently
/// becoming reference equality.
#[test]
fn equality_on_an_unsettled_operand_is_still_refused() {
    assert_refused_by_codegen(
        "state xs is client Whole starting empty\n\
         state same is client Truth from xs is xs\n\
         view\n\
         \x20   Text same\n",
        "by identity",
    );
}

/// `===` on a record compares identity, and the runtime has no structural
/// comparison to fall back on, so it is refused rather than quietly
/// answering a different question (§16.3.3, §16.7 item 2).
#[test]
fn comparing_two_records_is_refused_rather_than_compared_by_identity() {
    assert_refused_by_codegen(
        "record Point\n\
         \x20   x is Whole\n\
         state a is client Point starting Point with x is 1\n\
         state same is client Truth from a is a\n\
         view\n\
         \x20   Text same\n",
        "by identity",
    );
}

// --- records, choices and the collection literals -------------------------

/// §16.3: a record is a plain object, and its fields are emitted in
/// declaration order however the literal wrote them, so every instance
/// shares one hidden class (§16.7 item 9).
#[test]
fn a_record_literal_is_an_object_in_declaration_order() {
    let bundle = compile_source(
        "record Todo\n\
         \x20   id    is Whole\n\
         \x20   title is Text\n\
         \x20   done  is Truth\n\
         state one is client Todo starting Todo with done is no, title is \"x\", id is 1\n\
         view\n\
         \x20   Text one.title\n",
    );
    assert!(
        bundle
            .client_js
            .contains("signal({ id: 1, title: 'x', done: false })"),
        "{}",
        bundle.client_js
    );
}

/// **An object literal cannot open an arrow function's concise body.**
///
/// `() => { id: 1 }` is legal JavaScript and means a function whose body
/// is a block containing a label, so a `derived` holding a record
/// returned `undefined` and nothing said so. Precedence does not catch
/// this: an object literal binds as tightly as anything can, and the
/// hazard is the leading `{` alone. `js::arrow_body` answers it at every
/// site that writes `() => …`.
#[test]
fn a_derived_record_is_parenthesised_so_it_is_not_read_as_a_block() {
    let bundle = compile_source(
        "record Todo\n\
         \x20   id    is Whole\n\
         \x20   title is Text\n\
         state seed is client Whole starting 1\n\
         state one is client Todo from Todo with id is seed, title is \"x\"\n\
         view\n\
         \x20   Text one.title\n",
    );
    assert!(
        bundle.client_js.contains("derived(() => ({ id:"),
        "{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("derived(() => { id:"),
        "the bare object literal would be read as a block: {}",
        bundle.client_js
    );
}

/// §16.3: a choice value is `{ tag, fields }`, which is what `variant`
/// builds and what `when` dispatches on.
#[test]
fn a_variant_is_built_with_the_runtimes_variant_helper() {
    let bundle = compile_source(
        "choice Status\n\
         \x20   Active\n\
         \x20   Archived with reason is Text\n\
         state a is client Status starting Active\n\
         state b is client Status starting Archived with reason is \"old\"\n\
         view\n\
         \x20   Column\n\
         \x20       when a\n\
         \x20           Active           show Text \"active\"\n\
         \x20           Archived with r  show Text r\n\
         \x20       when b\n\
         \x20           Active           show Text \"active\"\n\
         \x20           Archived with r  show Text r\n",
    );
    let client = &bundle.client_js;
    assert!(client.contains("signal(variant('Active'))"), "{client}");
    assert!(
        client.contains("signal(variant('Archived', 'old'))"),
        "{client}"
    );
}

/// §14B.4. A `Map` is a JavaScript `Map`, not an object: an object would
/// coerce every key to a string.
#[test]
fn collection_literals_emit_an_array_and_a_map() {
    let bundle = compile_source(
        "state tags   is client List of Text          starting [\"red\", \"green\"]\n\
         state scores is client Map of Text to Whole  starting [\"a\" to 1, \"b\" to 2]\n\
         view\n\
         \x20   Column\n\
         \x20       each tag in tags\n\
         \x20           Text tag\n\
         \x20       Button \"drop\"\n\
         \x20           on click\n\
         \x20               remove \"a\" from scores\n",
    );
    let client = &bundle.client_js;
    assert!(client.contains("signal(['red', 'green'])"), "{client}");
    assert!(
        client.contains("signal(new Map([['a', 1], ['b', 2]]))"),
        "{client}"
    );
}

/// §14B.2's membership verbs. Both build a new collection: ZD values are
/// immutable and `signal.write` compares with `Object.is`, so mutating the
/// old one would defeat change detection.
///
/// `append` spreads and `remove` forces, and the asymmetry is the whole
/// of what the `append` *expression* costs the rest of the emitter: a
/// list a program built one element at a time is a chain until something
/// looks at it, spreading iterates it — which looks at it — and `.filter`
/// is an array method, which does not.
#[test]
fn append_and_remove_rebuild_the_collection_rather_than_mutating_it() {
    let bundle = compile_source(
        "state tags is client List of Text starting []\n\
         view\n\
         \x20   Column\n\
         \x20       Button \"add\"\n\
         \x20           on click\n\
         \x20               append \"red\" to tags\n\
         \x20       Button \"drop\"\n\
         \x20           on click\n\
         \x20               remove \"red\" from tags\n",
    );
    let client = &bundle.client_js;
    assert!(client.contains("setTags([...tags(), 'red'])"), "{client}");
    assert!(
        client.contains("setTags($force(tags()).filter(($e) => $e !== 'red'))"),
        "{client}"
    );
}

#[test]
fn removing_from_a_map_drops_the_entry_with_that_key() {
    let bundle = compile_source(
        "state scores is client Map of Text to Whole starting [\"a\" to 1]\n\
         view\n\
         \x20   Button \"drop\"\n\
         \x20       on click\n\
         \x20           remove \"a\" from scores\n",
    );
    assert!(
        bundle
            .client_js
            .contains("setScores(new Map([...scores()].filter(($e) => $e[0] !== 'a')))"),
        "{}",
        bundle.client_js
    );
}

// --- the other artifacts --------------------------------------------------

#[test]
fn the_index_page_loads_the_stylesheet_and_calls_main() {
    let bundle = compile_example("examples/counter.zd");
    // The name carries a content hash (#137), and the bundle is what
    // says which one — a test that spelled it would be asserting against
    // its own copy of the rule rather than against the emitter's.
    assert!(page(&bundle).contains(&format!(
        r#"<link rel="stylesheet" href="./{}">"#,
        bundle.styles_path
    )));
    // The opening tag: a document ships with its first paint inside.
    assert!(page(&bundle).contains(r#"<div id="app">"#));
    // The mount call is in `boot.js` and not in the page, so the page can
    // carry a policy with no inline-script exception (#146). Both halves
    // are asserted: a page loading a module nobody wrote renders nothing.
    assert!(page(&bundle).contains(r#"<script type="module" src="./boot.js"></script>"#));
    assert!(bundle
        .boot_js
        .as_deref()
        .expect("a program with a `view` emits its boot module")
        .contains("main(document.getElementById('app'))"));
    // `<html>` and `<body>` are written out rather than left implicit,
    // because `lang` belongs on the first of them.
    assert!(page(&bundle).contains(r#"<html lang="en">"#));
    assert!(page(&bundle).contains("<body>"));
    assert!(page(&bundle)
        .contains(r#"<meta name="viewport" content="width=device-width, initial-scale=1">"#));
    // With no metadata written, the title is the source file's stem.
    assert!(page(&bundle).contains("<title>test</title>"));
}

#[test]
fn a_view_carries_the_documents_metadata() {
    let bundle = compile_source(
        "view title is \"Field notes\", description is \"What I have been reading\", language is \
         \"en-GB\"\n    Paragraph \"hello\"\n",
    );
    assert!(
        page(&bundle).contains("<title>Field notes</title>"),
        "{}",
        page(&bundle)
    );
    assert!(
        page(&bundle).contains(r#"<meta name="description" content="What I have been reading">"#)
    );
    assert!(page(&bundle).contains(r#"<html lang="en-GB">"#));
}

#[test]
fn document_metadata_is_escaped_where_it_lands() {
    let bundle = compile_source(
        "view title is \"Tags & <script>\", description is \"a > b & c\"\n    Text \"x\"\n",
    );
    assert!(
        page(&bundle).contains("<title>Tags &amp; &lt;script&gt;</title>"),
        "{}",
        page(&bundle)
    );
    // §16.3.5: `&` and `<` and `>` in text position, `&` and `"` in
    // attribute position. A `>` inside a quoted attribute value ends
    // nothing, so escaping it would only make the output noisier.
    assert!(
        page(&bundle).contains(r#"content="a > b &amp; c""#),
        "{}",
        page(&bundle)
    );
}

#[test]
fn a_view_refuses_metadata_it_has_no_meaning_for() {
    let messages = resolve_refusals("view keywords is \"a, b\"\n    Text \"x\"\n");
    assert!(
        messages.iter().any(|m| m.contains("`keywords`")),
        "{messages:?}"
    );
}

/// The document is written when the bundle is built, so there is no run
/// time at which a computed title could be evaluated. A title that silently
/// never updated would be worse than one the compiler refuses.
#[test]
fn document_metadata_must_be_written_rather_than_computed() {
    let messages = resolve_refusals(
        "state name is client Text starting \"a\"\nview title is name\n    Text name\n",
    );
    assert!(
        messages.iter().any(|m| m.contains("text written here")),
        "{messages:?}"
    );
}

#[test]
fn asset_stylesheets_are_linked_after_the_generated_one() {
    let program = zdc_parser::parse("view\n    Text \"x\"\n").expect("parses");
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect("resolves");
    let split = zdc_graph::split(&hir);
    let verdict = zdc_graph::ifc(&hir, &split);
    let types = zdc_types::check(&hir, &split).expect("typechecks");
    let options = zdc_codegen::Options::new("test.zd", "test")
        .with_stylesheets(vec!["/assets/site.css".to_string()]);
    let cleared = verdict
        .clearance()
        .unwrap_or_else(|| panic!("flow: {}", verdict.diagnostics[0].message));
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &types,
        cleared,
    };
    let bundle = zdc_codegen::compile(&inputs, &options).expect("compiles");

    let generated = page(&bundle)
        .find(&format!(r#"href="./{}""#, bundle.styles_path))
        .expect("the generated stylesheet is linked");
    let asset = page(&bundle)
        .find(r#"href="/assets/site.css""#)
        .expect("the asset stylesheet is linked");
    assert!(
        generated < asset,
        "a program's own rules must come after the base classes, so they win \
         without an `!important`:\n{}",
        page(&bundle)
    );
}

/// The manifest is client-readable, so it may name endpoints and placements
/// and nothing else — never an initializer, never an environment key.
///
/// `origins` joins that list under §16.3.12 assertion C (#238): the browser
/// is about to fetch every entry in it, so naming them tells the client
/// nothing it will not see. It is empty here, and present while empty, so a
/// reader of the manifest can tell "this page fetches no remote module"
/// apart from "this compiler does not say".
#[test]
fn the_manifest_records_placements_and_no_initializers() {
    let bundle = compile_example("examples/counter.zd");
    assert_eq!(
        bundle.manifest_json.trim(),
        r#"{"entry":"client.js","functions":[],"durable":[],"transactions":[],"origins":[],"connect":[],"signals":{"count":"client","doubled":"client"}}"#
    );
}

/// Every line of a source that calls the platform's error channel.
///
/// **Two spellings, because the runtime documents two and this used to
/// look for one** (#22). `runtime/rpc.js` says it in as many words:
/// *"`reportError` is the platform's own 'this went wrong and nobody
/// caught it' channel — it reaches `window.onerror` and error-reporting
/// services the way a genuinely uncaught exception would. `console.error`
/// is the fallback for runtimes that predate it."* A gate that greps
/// `console.` therefore measures the fallback and misses the channel, and
/// it did: `dom.js` and `keys.js` each report a throwing handler through
/// `reportError` and neither was visible to the test that claimed one
/// logging call existed in the whole runtime.
///
/// Comment lines are dropped, because only a call site logs. The check is
/// per line rather than over the whole text so the caller can say *where*.
fn logging_lines(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter(|line| {
            let code = line.trim_start();
            !code.starts_with("//") && !code.starts_with('*') && !code.starts_with("/*")
        })
        .filter(|line| line.contains("console.") || line.contains("reportError("))
        .collect()
}

/// §14G.1.3(c)'s sink 5, the platform log, has a `SinkSite` variant that
/// nothing constructs — and it is unconstructible rather than merely
/// unconstructed only for as long as nothing writes a program's values to
/// a log. Asserted over the emitted text, because emission is what would
/// introduce one and the flow pass would not see it.
///
/// **Generated code, not the runtime it imports.** The distinction is the
/// whole of `Sink::producer`'s second condition: a logging call the
/// *emitter* writes lands in a function bundle, where the platform is
/// doing the logging and the visitor sees none of it. What the runtime
/// does is the test below.
#[test]
fn nothing_emitted_writes_to_a_platform_log() {
    let mut scanned = 0;
    for path in [
        "examples/hello.zd",
        "examples/counter.zd",
        "examples/guestbook.zd",
    ] {
        let bundle = compile_example(path);
        let mut sources = vec![("client.js".to_string(), bundle.client_js.clone())];
        sources.extend(
            bundle
                .functions
                .iter()
                .map(|f| (f.path.clone(), f.source.clone())),
        );
        for (name, source) in sources {
            assert!(
                logging_lines(&source).is_empty(),
                "{path} emits a logging call in {name}, which is sink 5 and nothing checks it"
            );
            scanned += 1;
        }
    }
    // A bundle that emitted nothing would satisfy the loop above without
    // reading a byte, which is the shape this suite exists to refuse.
    assert!(scanned >= 3, "only {scanned} emitted sources were read");
}

/// **Three logging calls in the shipped runtime, each named here.**
///
/// This test used to be called
/// `the_runtimes_only_logging_call_is_the_replaceable_failure_sink`, and
/// the claim in its name was false (#22). It looked for `console.`, and
/// the runtime's own channel is `reportError` — so `dom.js` and
/// `keys.js`, which report a throwing handler through it, were invisible
/// to the gate that said they did not exist. `ifc.rs` repeated the same
/// sentence about the same bytes, which is how one wrong measurement
/// becomes a documented guarantee.
///
/// All three hand a value to the **visitor's own browser**, which is the
/// reader sinks 1, 2, 4 and 7 already exist for, so none of them is sink
/// 5: §5.3a's medium is the platform's log, written about a server
/// execution the visitor never sees. A fourth call, or one of these three
/// moving into a *function* bundle, is a different matter and fails here.
///
/// Scanned over the runtime directory rather than over a bundle's linked
/// set, because a module no example happens to link is still a module the
/// emitter can ship.
#[test]
fn every_logging_call_in_the_shipped_runtime_is_named_here() {
    // `dom-shim.js` and `*.test.js` are the runtime's own test plumbing:
    // the shim *defines* `reportError` for an engine that has none, and
    // is never shipped. `runtime_files` is the list of what is, and it
    // names neither.
    let directory = repository_path("crates/zdc-runtime/runtime");
    let mut logging: Vec<String> = Vec::new();
    let mut scanned = 0;
    for entry in std::fs::read_dir(&directory).expect("the runtime directory") {
        let path = entry.expect("a directory entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if !name.ends_with(".js") || name.ends_with(".test.js") || name == "dom-shim.js" {
            continue;
        }
        scanned += 1;
        let source = std::fs::read_to_string(&path).expect("a readable runtime module");
        if !logging_lines(&source).is_empty() {
            logging.push(name);
        }
    }
    logging.sort();

    assert!(
        scanned >= 13,
        "the emitter can ship thirteen runtime modules; this read {scanned}"
    );
    assert_eq!(
        logging,
        ["dom.js", "keys.js", "rpc.js"],
        "the shipped runtime's logging calls are these three, each reaching the visitor's own \
         browser. A module joining the list is a new place a value is copied to, and it has to \
         be ruled on rather than added here"
    );

    // And `rpc.js`'s stays inside the sink a host page can replace, which
    // is the property `setFailureSink` exists to offer.
    let rpc = zdc_runtime::RPC_JS;
    let (before, after) = rpc
        .split_once("function defaultFailureSink(")
        .expect("the failure sink is where the exception lives");
    // Where the sink ends, found by **counting braces** rather than by
    // looking for the first `}` in the first column.
    //
    // The column was what this test used to look for, and #135 is what
    // showed that to be a fact about indentation rather than about the
    // code. Minified, `} else if (…)` sits in the first column like every
    // other line, so the scan stopped half way through the function and
    // counted the sink's *own* two `console.error` lines as being outside
    // it. Nothing had moved and nothing had been renamed.
    //
    // Counting braces reads the same answer out of both forms, so this
    // gate no longer depends on how the file happens to be laid out.
    let (sink, outside_the_sink) = {
        let mut depth = 0usize;
        let mut end = after.len();
        let mut opened = false;
        for (at, byte) in after.bytes().enumerate() {
            match byte {
                b'{' => {
                    depth += 1;
                    opened = true;
                }
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if opened && depth == 0 {
                        end = at + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        (&after[..end], format!("{before}\n{}", &after[end..]))
    };
    // **The guard against passing over the wrong region.** A brace scan
    // that ran to the end of the file would leave `outside_the_sink`
    // empty, and an empty haystack contains no `console.` — which is the
    // shape of a test that has stopped testing. So the sink is required
    // to be what it claims: the one place the call actually lives.
    assert!(
        sink.contains("console.error(error)"),
        "the brace scan did not land on the failure sink's body; this test \
         is looking at the wrong region and would pass whatever rpc.js did"
    );
    assert!(
        outside_the_sink.contains("export function reportFailure"),
        "the region outside the sink lost the rest of the module, so there \
         is nothing left for this test to inspect"
    );
    assert_eq!(
        logging_lines(&outside_the_sink),
        Vec::<&str>::new(),
        "rpc.js writes to a log outside `defaultFailureSink`, which a host page cannot replace"
    );
}

fn linked(example: &str) -> Vec<&'static str> {
    let bundle = compile_example(example);
    zdc_codegen::runtime_files(&bundle.runtime, zdc_codegen::Mode::Release)
        .into_iter()
        .map(|(path, _)| path)
        .collect()
}

/// §16.3.1: a bundle ships nothing it does not use — **as bytes**, not
/// only as import lines.
///
/// This used to assert the constant list of all five files, which is what
/// let `hello.zd` ship `rpc.js`, `store.js` and `wire.js` beside a
/// `client.js` that imports neither. The claim is about what lands in
/// `dist/`, so that is what is asserted.
#[test]
fn a_client_only_bundle_ships_no_networking_runtime() {
    let names = linked("examples/counter.zd");
    assert_eq!(names, ["runtime/dom.js", "runtime/signal.js"]);
}

/// `elements.js` is deliberately absent from every bundle: generated code
/// never imports it. It remains the reference implementation the parity
/// test checks the compiler's shape table against.
#[test]
fn no_bundle_ships_the_element_library() {
    for example in [
        "examples/counter.zd",
        "examples/guestbook.zd",
        "examples/todo.zd",
    ] {
        assert!(
            !linked(example).contains(&"runtime/elements.js"),
            "{example} shipped the element library"
        );
    }
}

/// A `durable` program links the live-sync half — and the two modules it
/// imports in turn, which is the part a hand-written list gets wrong.
#[test]
fn a_durable_bundle_ships_the_transitive_closure_of_what_it_imports() {
    let names = linked("examples/guestbook.zd");
    for wanted in [
        "runtime/signal.js",
        "runtime/dom.js",
        "runtime/rpc.js",
        "runtime/store.js",
        // Named by neither `client.js` nor the split: `rpc.js` and
        // `store.js` both import it, so omitting it would break the page
        // at load with a 404 no test of the import list would catch.
        "runtime/wire.js",
    ] {
        assert!(
            names.contains(&wanted),
            "{wanted} is missing from {names:?}"
        );
    }
}

// --- what `zdc check` accepts and `zdc build` does not ---------------------

/// A program comparing two `Text` values that `zdc check` accepts and
/// `zdc build` refuses.
///
/// `without` is inferred generically: the checker leaves `n` and `gone` at
/// an unresolved variable and never unifies them against the one call
/// site, which passes `List of Text` and `Text`. So `zdc check` exits 0
/// (the table holds `n is not gone : Truth` with both operands recorded as
/// `Type::Unknown`), and then §16.7 item 2's operand rule reads `Unknown`
/// here and refuses — reporting that `is` compares "a type that is not
/// known here" and advising the author to compare a `Text` field instead,
/// which is exactly what the source already does.
///
/// Replacing `gone` with the literal `"a"` makes the same program build,
/// which is what isolates this to the parameter rather than to `keep`.
const POLYMORPHIC_COMPARISON: &str = r#"state names is client List of Text starting ["a", "b", "c"]

function without with all, gone
    from all
    keep each n where n is not gone

view
    Column
        each n in names
            Text n
        Button "drop"
            on click
                set names to without with names, "b"
"#;

/// **This was `#[ignore]`d as a known defect, and the defect is closed.**
///
/// It demonstrated that the front end and the emitter disagreed about the
/// same program: `zdc check` accepted it and `zdc build` refused it.
/// Closing the gap meant picking one of three, and two of them landed —
/// `zdc check` now runs the emitter, so the two commands cannot answer
/// differently about *any* program, and §16.7's operand rule types the
/// compared parameter, so this one compiles. The `#[ignore]` is removed
/// rather than kept: a rationale that says the compiler disagrees with
/// itself is a false statement about the repository once it does not, and
/// leaving it would hide that the gap shut.
///
/// It stays as a regression test, in the shape it was written in. If a
/// later change makes the emitter refuse a program the checker accepts,
/// this is where it fails.
#[test]
fn a_comparison_the_checker_accepts_must_also_emit() {
    let bundle = support::try_compile(POLYMORPHIC_COMPARISON, "polymorphic.zd");
    assert!(
        bundle.is_ok(),
        "the checker accepted this program, so the emitter must too; got: {:?}",
        bundle
            .err()
            .map(|errors| errors.into_iter().map(|e| e.message).collect::<Vec<_>>())
    );
}

/// The half that always passed, kept beside it so the test above is pinned
/// to the parameter and not to `keep`, to `is not`, or to lists.
#[test]
fn the_same_comparison_against_a_literal_emits() {
    let literal = POLYMORPHIC_COMPARISON
        .replace("with all, gone", "with all")
        .replace("n is not gone", "n is not \"a\"")
        .replace("without with names, \"b\"", "without with names");
    support::try_compile(&literal, "literal.zd")
        .expect("comparing against a literal must still emit");
}

// --- one module, two pipelines --------------------------------------------

/// Two pipeline runs in one block used to emit `let $p` twice at the same
/// brace depth, which is a JavaScript `SyntaxError`.
///
/// This is a wrong-code bug rather than a wrong-value one, and the
/// difference is what makes it worth a test that loads the module: the
/// program compiles, `zdc build` exits 0, and the *whole bundle* then fails
/// to parse — so the failure is a blank page and an error in a console
/// nobody is looking at, not a wrong number on the screen. Being unreachable
/// after the first run's `return` is no defence: a redeclaration in the same
/// scope is rejected before a line of the module runs.
///
/// The assertion is therefore that the engine accepts the module and the
/// function still computes both pipelines, not that the source contains any
/// particular name. `run` evaluates the emitted module and panics if it does
/// not parse, which is exactly the failure being pinned.
///
/// The view renders `shown`, and has to: §16.3.12's client walk is rooted
/// at the document's nodes, so a signal no node reads is not emitted at
/// all and there would be no module to fail to parse. The fixture used to
/// render a literal, which was enough before the walk was narrowed.
#[test]
fn two_pipeline_runs_in_one_block_emit_a_module_that_loads() {
    let bundle = compile_source(
        "state items is client List of Whole starting [3, 1, 2]\n\
         state cutoff is client Truth starting yes\n\
         state shown is client List of Whole from twice with items\n\
         \n\
         function twice with all\n\
         \x20   if cutoff\n\
         \x20       from all\n\
         \x20       take first 1\n\
         \x20       each x in all\n\
         \x20           give all\n\
         \x20       from all\n\
         \x20       take first 2\n\
         \x20   give all\n\
         \n\
         view\n\
         \x20   each n in shown\n\
         \x20       Text n\n",
    );

    let mut context = context(false);
    let shown = run(&mut context, &bundle.client_js, "shown().join(',');");
    assert_eq!(
        shown, "3",
        "the first pipeline run is the one that returns, and the module has to load for it to"
    );
}

/// **The manifest names the program's signals and not the compiler's.**
///
/// A `test` claim desugars to a `static` signal called `$testN`, and those
/// were reaching the manifest: a file with two claims published two static
/// signals a reader of that file could not find, to a deploy target with
/// no use for them. §16.3.12 assertion C governs what the manifest may
/// carry; this is the other half — what it may not invent.
///
/// `$` is what makes the filter exact rather than a heuristic. An
/// identifier is `[\p{XID_Start}_][\p{XID_Continue}]*` and `$` is in
/// neither class, so a name beginning with one cannot have come from
/// source.
#[test]
fn the_manifest_omits_signals_the_compiler_synthesised() {
    let bundle = compile_source(
        "function twice of n\n\
         \x20   give n * 2\n\
         \n\
         state shown is client Whole starting 3\n\
         \n\
         test \"twice of two is four\"\n\
         \x20   expect twice of 2 is 4\n\
         \n\
         test \"twice of three is six\"\n\
         \x20   expect twice of 3 is 6\n\
         \n\
         view\n\
         \x20   Text (text of shown)\n",
    );
    assert!(
        !bundle.manifest_json.contains("$test"),
        "a synthesised claim reached the manifest:\n{}",
        bundle.manifest_json
    );
    // The program's own signal is still there, so the filter removed the
    // right thing rather than everything.
    assert!(
        bundle.manifest_json.contains(r#""shown":"client""#),
        "the program's own signal is missing:\n{}",
        bundle.manifest_json
    );
}
