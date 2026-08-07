//! Components, evaluated in the embedded engine.
//!
//! §14D.1 makes three claims that only running the emitted module can
//! settle, because each of them is about what happens after mount rather
//! than about what the compiler accepted:
//!
//!   1. A component renders — its body lands where the call site was, and
//!      `Row` and `Counter` are indistinguishable in the output.
//!   2. Component-local state is scoped **per instance**. Two `Counter`s
//!      count independently, and nothing in the program says so; it falls
//!      out of the state being declared inside the component.
//!   3. `children` land where the *body* puts them, not where they were
//!      written — `Panel` puts them inside its `Details`, after the
//!      `Summary`, and that is where they appear.
//!
//! There is no hand-written demo to compare against, as there is for
//! `hello.zd` and `counter.zd`: the demo pages predate components
//! entirely. So this asserts the rendered DOM directly, which is what a
//! browser would have shown.

mod support;

use support::{compile_example, compile_source, context, run};

/// Click the two counters different numbers of times.
///
/// Deliberately asymmetric: clicking both the same number of times would
/// pass just as well against one shared signal as against two.
///
/// The panel is no longer driven, because there is nothing to drive: it is
/// a `details`, so opening it is the browser's own behaviour and there is
/// no handler, no signal and no conditional region between the click and
/// the content. What is asserted about it is where its children landed.
const DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
const $frames = [serialize($host)];
const $buttons = () => walk($host).filter((n) => n.tagName === 'button');
const $more = () => $buttons().filter((n) => serialize(n).includes('>more<'));

$more()[0].fire('click');
$more()[0].fire('click');
$more()[0].fire('click');
$more()[1].fire('click');
$frames.push(serialize($host));
"#;

fn frames(module: &str) -> Vec<String> {
    let mut context = context(false);
    run(
        &mut context,
        module,
        &format!("{DRIVER}\n$frames.join('\\n')"),
    )
    .lines()
    .map(str::to_string)
    .collect()
}

#[test]
fn a_component_renders_where_it_is_written() {
    let frames = frames(&compile_example("examples/disclosure.zd").client_js);

    // Each `Counter` produced its own `Column`, with the label it was
    // given and its own count beside it.
    assert!(
        frames[0].contains("<span>left</span><span>0</span>"),
        "the first instance must render its label and its count:\n{}",
        frames[0]
    );
    assert!(
        frames[0].contains("<span>right</span><span>0</span>"),
        "the second instance must render too:\n{}",
        frames[0]
    );
    // A component is written where a built-in element is, so its body's
    // `Column` is an ordinary `zd-col` in the output with nothing marking
    // it as having come from a component.
    assert_eq!(
        frames[0].matches("zd-col").count(),
        3,
        "the view's own Column plus one per `Counter`; `Panel` is a \
         `Details` and adds none:\n{}",
        frames[0]
    );
}

/// The property §14D.1 exists to state: state inside a component belongs
/// to the instance, not to the component.
#[test]
fn two_instances_of_one_component_keep_separate_state() {
    let frames = frames(&compile_example("examples/disclosure.zd").client_js);

    assert!(
        frames[1].contains("<span>left</span><span>3</span>"),
        "three clicks on the first counter must reach only it:\n{}",
        frames[1]
    );
    assert!(
        frames[1].contains("<span>right</span><span>1</span>"),
        "one click on the second counter must reach only it:\n{}",
        frames[1]
    );
}

/// The nodes nested at the call site are placed by the component naming
/// `children` in its body, and the body puts them inside its `Details`
/// after the `Summary` — not where they were written, which is under the
/// `Panel` call itself.
#[test]
fn children_land_where_the_body_puts_them() {
    let frames = frames(&compile_example("examples/disclosure.zd").client_js);

    assert!(
        frames[0].contains(
            "<details><summary>details</summary>\
             <div class=\"zd-row\"><span>inside the panel</span></div></details>"
        ),
        "the call site's nodes must land after the summary, inside the disclosure:\n{}",
        frames[0]
    );
    // And the driver really ran: the counters moved between the frames,
    // so a `Panel` that had rendered nothing at all would not pass above
    // by the page having been empty.
    assert!(
        frames[1].contains("<span>left</span><span>3</span>"),
        "the rest of the page must still be reacting:\n{}",
        frames[1]
    );
}

/// A component instance inside an `each` gets one signal per *row*, not
/// one per call site. The row closure runs once per item, so the
/// declaration inside it does too.
#[test]
fn a_component_inside_a_list_gets_one_signal_per_row() {
    let bundle = compile_source(
        "component Tally with label\n\
         \x20   state count is client Whole starting 0\n\
         \x20   Row\n\
         \x20       Text label\n\
         \x20       Text count\n\
         \x20       Button \"tick\"\n\
         \x20           on click\n\
         \x20               add 1 to count\n\
         state names is client List of Text starting [\"a\", \"b\", \"c\"]\n\
         view\n\
         \x20   Column\n\
         \x20       each name in names\n\
         \x20           Tally name\n",
    );

    // The declaration is inside the row closure, which is what makes it
    // per row rather than per call site.
    let client = &bundle.client_js;
    let row = client
        .split("eachInto(")
        .nth(1)
        .expect("the list emits an `eachInto`");
    assert!(
        row.contains("const [count, setCount] = signal(0);"),
        "the signal must be declared inside the row closure:\n{client}"
    );
    assert_eq!(
        client.matches("signal(0)").count(),
        1,
        "one declaration, evaluated once per row:\n{client}"
    );

    let mut context = context(false);
    let frames = run(
        &mut context,
        client,
        r#"
const $host = document.createElement('div');
main($host);
const $ticks = () => walk($host).filter((n) => n.tagName === 'button');
$ticks()[1].fire('click');
$ticks()[1].fire('click');
serialize($host)
"#,
    );

    assert!(
        frames.contains("<span>a</span><span>0</span>"),
        "the first row must be untouched:\n{frames}"
    );
    assert!(
        frames.contains("<span>b</span><span>2</span>"),
        "only the row that was clicked must count:\n{frames}"
    );
    assert!(
        frames.contains("<span>c</span><span>0</span>"),
        "and the third must be untouched too:\n{frames}"
    );
}

/// Colourlessness, from the emission's side: a component leaves nothing of
/// itself behind. There is no function per component, no wrapper element,
/// and no runtime dispatch — the body is simply where the call site was.
#[test]
fn a_component_emits_no_runtime_of_its_own() {
    let client = compile_example("examples/disclosure.zd").client_js;
    assert!(
        !client.contains("function Counter") && !client.contains("Counter("),
        "a component is not a function in the output:\n{client}"
    );
    assert!(
        !client.contains("function Panel") && !client.contains("Panel("),
        "{client}"
    );
}
