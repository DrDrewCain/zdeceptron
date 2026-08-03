//! What the emitter produces, and what it refuses to produce.
//!
//! The refusals matter as much as the emissions. Every construct this
//! milestone cannot compile correctly has a test asserting it is rejected
//! with a message naming what is missing — a program that compiles to
//! something broken is worse than one that refuses.

mod support;

use support::{compile_example, compile_source, context, refusals, run};

/// §16.4's worked emission for `hello.zd`, verbatim.
const HELLO: &str = r#"// zdc 0.1.0 · examples/hello.zd · generated, do not edit
import { signal } from './runtime/signal.js';
import { bindAttr, bindText, mount, on, template } from './runtime/dom.js';

const $t0 = template('<div class="zd-col"><h2>Hello, ZDeceptron</h2><input type="text" placeholder="your name"><span> </span></div>');

const [name, setName] = signal('world');

export function main(container) {
  const $r = $t0();
  const $n0 = $r.firstChild;
  const $n1 = $n0.firstChild.nextSibling;
  const $n2 = $n1.nextSibling;
  bindAttr($n1, 'value', name);
  on($n1, 'input', (e) => setName(e.target.value));
  bindText($n2.firstChild, name);
  return mount($r, container);
}
"#;

/// §16.4's worked emission for `counter.zd`, verbatim.
const COUNTER: &str = r#"// zdc 0.1.0 · examples/counter.zd · generated, do not edit
import { derived, signal } from './runtime/signal.js';
import { bindText, mount, on, template } from './runtime/dom.js';

const $t0 = template('<div class="zd-col"><h2>Counter</h2><span> </span><span> </span><div class="zd-row"><button type="button">minus one</button><button type="button">plus one</button><button type="button">reset</button></div></div>');

const [count, setCount] = signal(0);
const doubled = derived(() => count() * 2);

export function main(container) {
  const $r = $t0();
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
  return mount($r, container);
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
    let messages = refusals(source);
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
    // §16.4's exact line: the browser ships the amount and asks.
    assert!(
        bundle.client_js.contains("$call('visits.incr', 1)"),
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

/// The four checked-in examples that write `Row item.name` disagree with
/// `elements.js`, and §16.3.6 escalates that to a language decision rather
/// than letting codegen invent the semantics.
#[test]
fn a_leading_argument_to_row_is_refused_and_names_the_open_decision() {
    assert_refused(
        "view\n    Row \"label\"\n        Text \"a\"\n",
        "until that is ratified",
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

/// §16.7 item 5 is only half a type question. The checker does say which
/// container this is; what is missing is the runtime helper that builds the
/// `Option of T` §5.4 promises, and that is §14F's standard library.
#[test]
fn at_is_refused_because_the_runtime_has_no_option_to_build() {
    assert_refused(
        "state xs is client List of Whole starting []\n\
         state one is client Option of Whole from xs at 0\n\
         view\n\
         \x20   when one\n\
         \x20       Some with value show Text value\n\
         \x20       None            show Text \"none\"\n",
        "Option of T",
    );
}

#[test]
fn a_mutation_through_a_path_is_refused_naming_the_open_question() {
    assert_refused(
        "state scores is client Map of Whole to Whole starting empty\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           add 1 to scores at 0\n",
        "§14B.3",
    );
}

#[test]
fn a_program_without_a_view_is_refused() {
    assert_refused("state a is client Whole starting 1\n", "no `view`");
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
    assert_refused(
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
    assert_refused(
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
        client.contains("setTags(tags().filter(($e) => $e !== 'red'))"),
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
    assert!(bundle
        .index_html
        .contains(r#"<link rel="stylesheet" href="./styles.css">"#));
    assert!(bundle.index_html.contains(r#"<div id="app"></div>"#));
    assert!(bundle
        .index_html
        .contains("main(document.getElementById('app'))"));
}

/// The manifest is client-readable, so it may name endpoints and placements
/// and nothing else — never an initializer, never an environment key.
#[test]
fn the_manifest_records_placements_and_no_initializers() {
    let bundle = compile_example("examples/counter.zd");
    assert_eq!(
        bundle.manifest_json.trim(),
        r#"{"entry":"client.js","functions":[],"durable":[],"signals":{"count":"client","doubled":"client"}}"#
    );
}

#[test]
fn the_runtime_files_a_bundle_links_against_exclude_the_element_library() {
    let names: Vec<&str> = zdc_codegen::runtime_files()
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert_eq!(
        names,
        ["runtime/signal.js", "runtime/dom.js", "runtime/rpc.js"]
    );
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

/// **Ignored: this fails, and fixing it is a design decision.**
///
/// It demonstrates that the front end and the emitter disagree about the
/// same program. Closing the gap means picking one of three: infer a
/// function's parameters monomorphically from its call sites, resolve the
/// operand type through the caller during emission, or run §16.7's operand
/// rule inside `zdc check` so the two commands answer alike. All three are
/// language decisions, not repairs, so the failure is recorded rather than
/// papered over.
#[test]
#[ignore = "known defect: `zdc check` accepts this and `zdc build` refuses it"]
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

/// The half that passes today, kept beside it so the ignored test above is
/// pinned to the parameter and not to `keep`, to `is not`, or to lists.
#[test]
fn the_same_comparison_against_a_literal_emits() {
    let literal = POLYMORPHIC_COMPARISON
        .replace("with all, gone", "with all")
        .replace("n is not gone", "n is not \"a\"")
        .replace("without with names, \"b\"", "without with names");
    support::try_compile(&literal, "literal.zd")
        .expect("comparing against a literal must still emit");
}
