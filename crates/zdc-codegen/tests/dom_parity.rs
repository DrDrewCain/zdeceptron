//! **The acceptance test.** Emitted output must produce the same DOM as the
//! hand-written demo files, and must keep producing it under interaction.
//!
//! `runtime/demo/hello.js` and `runtime/demo/counter.js` were verified
//! rendering in a real browser. They build the tree node by node through
//! `elements.js`; generated code clones a parsed template and binds at the
//! holes. The two are deliberately different strategies — that is the whole
//! decision in §16.1 — so the acceptance criterion is not a byte diff of the
//! JavaScript. It is that the DOM they produce is the same, from the same
//! starting state, and stays the same after a click and a keystroke.
//!
//! This is how a browser verification is inherited by a code generator
//! without a browser.

mod support;

use support::{compile_example, context, run};

/// Serialise the mounted tree after each step, holding nothing back:
/// comments, `input.value` and `checkbox.checked` are all included, because
/// a parity test that dropped them would compare equal on a missing anchor
/// or an unwritten form value.
const HELLO_DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
const $frames = [serialize($host)];
const $input = findTag($host, 'input');
$input.fire('input', { target: { value: 'Ada' } });
$frames.push(serialize($host));
"#;

const COUNTER_DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
const $frames = [serialize($host)];
const $buttons = walk($host).filter((n) => n.tagName === 'button');
if ($buttons.length !== 3) throw new Error('expected three buttons, got ' + $buttons.length);
for (const $index of [1, 1, 0, 2]) {
  $buttons[$index].fire('click');
  $frames.push(serialize($host));
}
"#;

const REPORT: &str = "$frames.join('\\n')";

fn demo(name: &str) -> String {
    std::fs::read_to_string(support::repository_path(&format!("runtime/demo/{name}")))
        .unwrap_or_else(|e| panic!("reading runtime/demo/{name}: {e}"))
}

/// The demo pages import `elements.js`; generated code must not, so the two
/// halves run in contexts that differ in exactly that.
fn frames(module: &str, driver: &str, elements: bool, epilogue: &str) -> String {
    let mut context = context(elements);
    run(
        &mut context,
        module,
        &format!("{driver}\n{epilogue}\n{REPORT}"),
    )
}

#[test]
fn hello_emits_the_dom_the_hand_written_demo_produces() {
    let emitted = compile_example("examples/hello.zd").client_js;

    let from_demo = frames(&demo("hello.js"), HELLO_DRIVER, true, "");
    let from_emission = frames(&emitted, HELLO_DRIVER, false, "");

    assert_eq!(
        from_emission, from_demo,
        "the emitted module must render and update exactly what the verified demo does"
    );
    assert!(
        from_demo.contains("<span>world</span>"),
        "the driver must actually have rendered something:\n{from_demo}"
    );
    assert!(
        from_demo.contains("<span>Ada</span>"),
        "typing must have reached the span:\n{from_demo}"
    );
}

/// A write straight into the signal, rather than through the DOM. The two
/// modules name their setter differently — the demo keeps the pair, the
/// emission destructures it — so this is the one step that cannot be one
/// shared script.
#[test]
fn hello_updates_identically_when_the_signal_itself_is_written() {
    let emitted = compile_example("examples/hello.zd").client_js;

    let from_demo = frames(
        &demo("hello.js"),
        HELLO_DRIVER,
        true,
        "name[1]('via signal'); $frames.push(serialize($host));",
    );
    let from_emission = frames(
        &emitted,
        HELLO_DRIVER,
        false,
        "setName('via signal'); $frames.push(serialize($host));",
    );

    assert_eq!(from_emission, from_demo);
    assert!(
        from_demo.contains(".value=\"via signal\""),
        "a signal write must reach the input as well as the span:\n{from_demo}"
    );
}

#[test]
fn counter_emits_the_dom_the_hand_written_demo_produces() {
    let emitted = compile_example("examples/counter.zd").client_js;

    let from_demo = frames(&demo("counter.js"), COUNTER_DRIVER, true, "");
    let from_emission = frames(&emitted, COUNTER_DRIVER, false, "");

    assert_eq!(
        from_emission, from_demo,
        "the emitted module must render and update exactly what the verified demo does"
    );

    // Plus, plus, minus, reset: 0 -> 2 -> 4 -> 2 -> 0 on both bindings.
    let doubled: Vec<&str> = from_demo.lines().collect();
    assert_eq!(doubled.len(), 5, "one frame per step:\n{from_demo}");
    for (frame, expected) in doubled.iter().zip(["0", "1", "2", "1", "0"]) {
        assert!(
            frame.contains(&format!("<span>{expected}</span>")),
            "expected count {expected} in:\n{frame}"
        );
    }
    for (frame, expected) in doubled.iter().zip(["0", "2", "4", "2", "0"]) {
        assert!(
            frame.contains(&format!("<span>{expected}</span>")),
            "expected doubled {expected} in:\n{frame}"
        );
    }
}

/// Generated code links against `signal.js` and `dom.js` only. If it ever
/// reached for `elements.js`, the parity above would be comparing a
/// strategy against itself and would stop meaning anything.
#[test]
fn the_emitted_module_never_imports_the_element_library() {
    for example in ["examples/hello.zd", "examples/counter.zd"] {
        let client = compile_example(example).client_js;
        assert!(
            !client.contains("elements.js"),
            "{example} must not import elements.js:\n{client}"
        );
        for built_in in zdc_codegen::BUILT_INS {
            assert!(
                !calls(&client, built_in),
                "{example} must not call the element constructor `{built_in}`:\n{client}"
            );
        }
    }
}

/// Whether `source` calls `name` as a whole identifier. Substring matching
/// would report `bindText(` as a call to `Text`.
fn calls(source: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    source.match_indices(&needle).any(|(at, _)| {
        at == 0
            || !source[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$')
    })
}

/// Template cloning's reason for existing: one parse, one clone, one
/// insertion, and an effect only where a signal is actually read.
#[test]
fn the_emitted_module_clones_a_template_rather_than_building_nodes() {
    let client = compile_example("examples/counter.zd").client_js;
    assert_eq!(client.matches("template(").count(), 1, "one template");
    assert!(!client.contains("createElement"), "no node construction");
    assert!(!client.contains("createTextNode"), "no text construction");
    // `count` and `doubled`, and nothing for the three constant labels.
    assert_eq!(client.matches("bindText(").count(), 2);
}
