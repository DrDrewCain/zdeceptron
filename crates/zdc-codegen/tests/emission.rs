//! What the emitter produces, and what it refuses to produce.
//!
//! The refusals matter as much as the emissions. Every construct this
//! milestone cannot compile correctly has a test asserting it is rejected
//! with a message naming what is missing — a program that compiles to
//! something broken is worse than one that refuses.

mod support;

use support::{compile_example, compile_source, context, refusals, run, try_compile};

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
fn a_server_signal_is_refused_by_name() {
    assert_refused(
        "state greeting is server Text starting \"hi\"\nview\n    Text \"x\"\n",
        "client bundle only",
    );
}

#[test]
fn a_durable_signal_is_refused_by_name() {
    assert_refused(
        "state visits is durable Whole starting 0\nview\n    Text \"x\"\n",
        "runtime/store.js",
    );
}

/// Codegen refuses to run without an information-flow verdict rather than
/// emit an unenforced guarantee (§16.3.12).
#[test]
fn a_secret_signal_is_refused_because_there_is_no_information_flow_pass() {
    assert_refused(
        "secret state key is server Text from environment \"K\"\nview\n    Text \"x\"\n",
        "zdc-graph",
    );
}

#[test]
fn a_view_position_when_is_refused_with_the_milestone_that_covers_it() {
    assert_refused(
        "state status is client Whole starting 1\n\
         view\n\
         \x20   when status\n\
         \x20       Loading show Spinner\n",
        "M5b",
    );
}

#[test]
fn a_view_position_each_is_refused_rather_than_emitting_a_frozen_list() {
    assert_refused(
        "state items is client Whole starting 1\n\
         view\n\
         \x20   each item in items\n\
         \x20       Text item\n",
        "never updates",
    );
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

#[test]
fn empty_and_at_are_refused_until_the_type_checker_can_say_what_they_are() {
    assert_refused(
        "state xs is client Whole starting empty\nview\n    Text \"a\"\n",
        "type checker",
    );
    assert_refused(
        "state xs is client Whole starting 1\n\
         state one is client Whole from xs at 0\n\
         view\n\
         \x20   Text one\n",
        "Option of T",
    );
}

#[test]
fn a_mutation_through_a_path_is_refused_naming_the_open_question() {
    assert_refused(
        "state scores is client Whole starting 1\n\
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

/// §16.7's two blocking gates. Neither is silently wrong; both are refused,
/// and both come through under `--unchecked`.
#[test]
fn addition_and_equality_are_gated_on_the_type_checker() {
    assert_refused(
        "state a is client Whole starting 1\n\
         state b is client Whole from a + 1\n\
         view\n\
         \x20   Text b\n",
        "addition and string concatenation",
    );
    assert_refused(
        "state a is client Whole starting 1\n\
         state b is client Truth from a is 1\n\
         view\n\
         \x20   Text b\n",
        "reference equality",
    );
}

#[test]
fn unchecked_emits_them_with_the_operators_the_specification_chose() {
    let source = "state a is client Whole starting 1\n\
                  state sum is client Whole from a + 1\n\
                  state same is client Truth from a is 1\n\
                  view\n\
                  \x20   Column\n\
                  \x20       Text sum\n\
                  \x20       Text same\n";
    let bundle = try_compile(source, "test.zd", true).expect("--unchecked compiles it");
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
    assert_eq!(names, ["runtime/signal.js", "runtime/dom.js"]);
}
