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

use support::{compile_example, compile_source, context, run};

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

/// `todo.zd` end to end: a `record`, a `choice`, collection and record
/// literals, `append` and `remove`, a node-position `when` and a
/// node-position `each`, evaluated in the embedded engine.
///
/// There is no hand-written demo to compare against — the demo pages
/// predate every construct here — so this asserts the rendered DOM
/// directly, which is what a browser would have shown.
const TODO_DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
const $frames = [serialize($host)];
const $buttons = () => walk($host).filter((n) => n.tagName === 'button');
const $click = (label) => {
  const $found = $buttons().find((n) => serialize(n).includes('>' + label + '<'));
  if ($found === undefined) throw new Error('no button labelled ' + label);
  $found.fire('click');
};
findTag($host, 'input').fire('input', { target: { value: 'ship it' } });
$click('add');
$frames.push(serialize($host));
$click('unfinished');
$frames.push(serialize($host));
$click('everything');
$buttons().filter((n) => serialize(n).includes('>delete<'))[0].fire('click');
$frames.push(serialize($host));
"#;

#[test]
fn todo_renders_its_list_and_appending_adds_a_row() {
    let emitted = compile_example("examples/todo.zd").client_js;
    let frames: Vec<String> = frames(&emitted, TODO_DRIVER, false, "")
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(frames.len(), 4, "one frame per step:\n{frames:#?}");

    // The seeded list renders both rows, in order.
    assert!(
        frames[0].contains("<span>write the parser</span>"),
        "the seeded list must render:\n{}",
        frames[0]
    );
    assert!(
        frames[0].contains("<span>write the checker</span>"),
        "{}",
        frames[0]
    );
    assert!(
        frames[0].contains("<span>showing everything</span>"),
        "the `when` arm must render:\n{}",
        frames[0]
    );

    // `append` adds exactly one row, and clears the draft.
    assert_eq!(rows(&frames[0]) + 1, rows(&frames[1]), "{}", frames[1]);
    assert!(
        frames[1].contains("<span>ship it</span>"),
        "the appended todo must be on the page:\n{}",
        frames[1]
    );
    assert!(
        frames[1].contains(".value=\"\""),
        "the draft must be cleared:\n{}",
        frames[1]
    );

    // The `when` rebuilds on a tag change, and the filter drops the row
    // whose `done` is `yes`.
    assert!(
        frames[2].contains("<span>showing what is left</span>"),
        "{}",
        frames[2]
    );
    assert!(
        !frames[2].contains("<span>write the parser</span>"),
        "a finished todo must be filtered out:\n{}",
        frames[2]
    );

    // `remove` drops exactly one row.
    assert_eq!(rows(&frames[1]) - 1, rows(&frames[3]), "{}", frames[3]);
}

/// How many todo rows a frame holds. Each row is a `zd-row` div, and the
/// two `zd-row`s the header and the filter bar contribute are constant.
fn rows(frame: &str) -> usize {
    frame.matches("zd-row").count() - 2
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

// --- the leading text slot on `Row` and `Column` ---------------------------
//
// §4.4 ratified a leading text slot for the two layout containers, which is
// what `components.zd`, `leaderboard.zd` and `voting-board.zd` had always
// written and what the compiler used to refuse. The shape it produces is a
// **bare text node before the children**, which is a different tree from a
// nested `Text` and is why the two phrasings do not collide under §4.1.
//
// Emitting it is not the claim; the claim is that it renders and stays
// reactive, so both tests below drive the module in the engine.

const SLOT_DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
const $frames = [serialize($host)];
const $click = (label) => {
  const $b = walk($host)
    .filter((n) => n.tagName === 'button')
    .find((n) => serialize(n).includes('>' + label + '<'));
  if (!$b) throw new Error('no button labelled ' + label);
  $b.fire('click');
  $frames.push(serialize($host));
};
"#;

/// A row's own text is a text node the row owns, ahead of its children, and
/// it re-renders in place when the signal behind it changes.
#[test]
fn a_rows_leading_text_renders_before_its_children_and_stays_reactive() {
    let bundle = compile_source(
        "state label is client Text starting \"first\"\n\
         state count is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Row label, padding is 8\n\
         \x20           Button \"rename\"\n\
         \x20               on click\n\
         \x20                   set label to \"second\"\n\
         \x20           Button \"tally\"\n\
         \x20               on click\n\
         \x20                   add 1 to count\n\
         \x20       Text count\n",
    );

    let mut context = context(false);
    let frames: Vec<String> = run(
        &mut context,
        &bundle.client_js,
        &format!("{SLOT_DRIVER}\n$click('rename');\n$click('tally');\n{REPORT}"),
    )
    .lines()
    .map(str::to_string)
    .collect();

    // The text is the row's own first child, not a `<span>`, and the two
    // buttons follow it inside the same row.
    assert!(
        frames[0].contains(
            "<div class=\"zd-row zd-s0\">first\
             <button type=\"button\">rename</button>\
             <button type=\"button\">tally</button></div>"
        ),
        "the leading text must be a bare text node before the children:\n{}",
        frames[0]
    );
    // Renaming rewrites that one text node and disturbs nothing else.
    assert!(
        frames[1].contains("<div class=\"zd-row zd-s0\">second<button"),
        "the leading text must track its signal:\n{}",
        frames[1]
    );
    assert!(
        frames[1].contains("<span>0</span>") && frames[2].contains("<span>1</span>"),
        "the rest of the view must keep working:\n{}\n{}",
        frames[1],
        frames[2]
    );
}

/// The shape `components.zd`, `leaderboard.zd` and `voting-board.zd` write:
/// a row inside an `each`, showing the binder. An `each` binder is reactive
/// (§16.3.3 R1), so this is the path a non-reactive leading slot would have
/// silently frozen.
#[test]
fn a_row_in_an_each_shows_its_binder_and_tracks_the_list() {
    let bundle = compile_source(
        "record Player\n\
         \x20   name is Text\n\
         state players is client List of Player starting [(Player with name is \"ada\"), \
         (Player with name is \"bo\")]\n\
         view\n\
         \x20   Column\n\
         \x20       each player in players\n\
         \x20           Row player.name, padding is 8\n\
         \x20               Button \"promote\"\n\
         \x20                   on click\n\
         \x20                       append (Player with name is \"new\") to players\n",
    );

    let mut context = context(false);
    let frames: Vec<String> = run(
        &mut context,
        &bundle.client_js,
        &format!("{SLOT_DRIVER}\n$click('promote');\n{REPORT}"),
    )
    .lines()
    .map(str::to_string)
    .collect();

    for name in ["ada", "bo"] {
        assert!(
            frames[0].contains(&format!(
                "<div class=\"zd-row zd-s0\">{name}<button type=\"button\">promote</button></div>"
            )),
            "each row shows its own binder:\n{}",
            frames[0]
        );
    }
    assert_eq!(
        frames[0].matches("zd-row").count(),
        2,
        "one row per player:\n{}",
        frames[0]
    );
    // Appending adds a third row, and the two that were already there keep
    // the text they were rendered with.
    assert_eq!(
        frames[1].matches("zd-row").count(),
        3,
        "the list must grow:\n{}",
        frames[1]
    );
    assert!(
        frames[1].contains(">ada<") && frames[1].contains("\">new<"),
        "the existing rows keep their text and the new one has its own:\n{}",
        frames[1]
    );
}
