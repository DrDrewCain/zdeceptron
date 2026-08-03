//! Event payloads, evaluated in the embedded engine.
//!
//! The claim under test is the one the gap analysis said was unreachable:
//! a handler can observe *what* happened, not merely that something did.
//! Only running the emitted module settles it, because every part of the
//! mechanism — the parameter, the accessor, the listener — is a byte of
//! generated JavaScript rather than a compiler verdict.
//!
//! It is driven exactly as `examples/disclosure.zd` is: the DOM shim, the
//! shipped runtime, the emitted module, and a script that fires events at
//! the rendered tree.

mod support;

use support::{compile_example, compile_source, context, refusals, run};

/// Type a key, then a chord, then commit on blur, then click.
///
/// The payloads carry values nothing else in the language could have
/// supplied: a key name, a modifier flag, a field's contents at the
/// moment focus left it, and a coordinate.
const DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
const $frames = [];
const $input = walk($host).find((n) => n.tagName === 'input');
const $button = walk($host).find((n) => n.tagName === 'button');

$input.fire('keydown', { key: 'j' });
$frames.push(html($host));

$input.fire('keydown', { key: 'k', ctrlKey: true });
$input.fire('keydown', { key: 'l', ctrlKey: true });
$frames.push(html($host));

$input.value = 'committed text';
$input.fire('blur');
$frames.push(html($host));

$button.fire('click', { clientX: 12, clientY: 34 });
$frames.push(html($host));
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
fn a_handler_observes_the_key_that_was_pressed() {
    let frames = frames(&compile_example("examples/events.zd").client_js);
    assert!(
        frames[0].contains("<span>last key: </span><span>j</span>"),
        "the keystroke's own key must reach the handler:\n{}",
        frames[0]
    );
}

/// A modifier is part of the payload, so a chord is a program.
#[test]
fn a_handler_observes_the_modifiers_that_were_held() {
    let frames = frames(&compile_example("examples/events.zd").client_js);
    assert!(
        frames[0].contains("<span>control chords: </span><span>0</span>"),
        "an unmodified keystroke is not a chord:\n{}",
        frames[0]
    );
    assert!(
        frames[1].contains("<span>control chords: </span><span>2</span>"),
        "two control-held keystrokes are two chords:\n{}",
        frames[1]
    );
}

/// The field's contents at the moment focus left it — the "commit on
/// blur" shape, which nothing but the two-way binding could reach before.
#[test]
fn a_handler_observes_a_fields_value_on_an_event_that_is_not_input() {
    let frames = frames(&compile_example("examples/events.zd").client_js);
    assert!(
        frames[1].contains("<span>committed: </span><span>nothing yet</span>"),
        "nothing is committed before focus leaves:\n{}",
        frames[1]
    );
    assert!(
        frames[2].contains("<span>committed: </span><span>committed text</span>"),
        "blur must carry the value:\n{}",
        frames[2]
    );
}

#[test]
fn a_handler_observes_where_a_click_landed() {
    let frames = frames(&compile_example("examples/events.zd").client_js);
    assert!(
        frames[3].contains("<span>x: </span><span>12</span>"),
        "the click's x coordinate must reach the handler:\n{}",
        frames[3]
    );
    assert!(
        frames[3].contains("<span> y: </span><span>34</span>"),
        "and its y coordinate:\n{}",
        frames[3]
    );
}

/// The two-way binding is unchanged by all of this, and is still the
/// emission §16.3.6's table writes out.
#[test]
fn the_two_way_binding_is_the_same_bytes_it_always_was() {
    let client =
        compile_source("state name is client Text starting \"\"\nview\n    Input name\n").client_js;
    assert!(
        client.contains("on($n0, 'input', (e) => setName(e.target.value));"),
        "{client}"
    );
}

/// The `on(...)` call a region attaches for the `input` event.
fn input_listener(client: &str) -> String {
    client
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("on($n0, 'input',"))
        .unwrap_or_else(|| panic!("no `input` listener was emitted:\n{client}"))
        .to_string()
}

/// The sugar and a hand-written handler are one implementation, not two.
///
/// `Input name` is `on input with e / set name to e.value` written by the
/// compiler: the event comes from `events::two_way_event` and the field
/// access from `events::accessor`, which is the same accessor the general
/// path reads. Emitting both and comparing the bytes is what would catch a
/// merge that gave the view emitter back its own idea of what `value`
/// means — the two would keep compiling and quietly disagree.
#[test]
fn the_sugar_and_a_hand_written_handler_emit_one_listener() {
    let sugar =
        compile_source("state name is client Text starting \"\"\nview\n    Input name\n").client_js;
    let hand = compile_source(
        "state name is client Text starting \"\"\n\
         view\n\
         \x20   Row\n\
         \x20       on input with e\n\
         \x20           set name to e.value\n",
    )
    .client_js;

    // The program's own binder moves aside because the sugar's parameter
    // holds that spelling (§16.3.2). Undoing exactly that rename is what
    // leaves the two listeners comparable; nothing else differs.
    let renamed = input_listener(&hand).replace("e$", "e");
    assert_eq!(
        input_listener(&sugar),
        renamed,
        "the two-way sugar and a hand-written `on input` must be one emission"
    );
}

/// The sugar's parameter is the one emitted name that is not `$`-prefixed,
/// so it is reserved instead. Before this, a program's own `e` was
/// shadowed inside every `Input`'s listener and the write went to the
/// wrong signal.
#[test]
fn a_program_may_declare_a_signal_called_e() {
    let client = compile_source(
        "state e is client Text starting \"a\"\n\
         state name is client Text starting \"\"\n\
         view\n\
         \x20   Column\n\
         \x20       Input name\n\
         \x20       Text e\n",
    )
    .client_js;
    assert!(
        client.contains("const [e$] = signal('a');"),
        "the program's own name must move aside:\n{client}"
    );
    assert!(
        client.contains("(e) => setName(e.target.value)"),
        "and the listener keeps the spelling §16.3.6 writes:\n{client}"
    );
}

// --- refusals -------------------------------------------------------------

/// The event set is closed, and the diagnostic offers the whole of it.
#[test]
fn an_event_the_language_does_not_know_is_refused() {
    let found = refusals(
        "state n is client Whole starting 0\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on pointermove with move\n\
         \x20           add 1 to n\n",
    );
    assert!(
        found
            .iter()
            .any(|m| m.contains("pointermove") && m.contains("`click`")),
        "{found:?}"
    );
}

/// A payload is not a bag. Reading a field this event does not carry is a
/// compile error naming the fields it does.
#[test]
fn a_field_the_payload_does_not_carry_is_refused() {
    let found = refusals(
        "state landed is client Text starting \"\"\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click with press\n\
         \x20           set landed to press.key\n",
    );
    assert!(
        found
            .iter()
            .any(|m| m.contains("PointerEvent") && m.contains("`x`")),
        "{found:?}"
    );
}

/// Binding an event that carries nothing is refused rather than allowed
/// and useless: §4.1 gives each construct one phrasing, and a binder with
/// no fields is a second way of writing `on submit`.
#[test]
fn binding_an_event_that_carries_nothing_is_refused() {
    let found = refusals(
        "state n is client Whole starting 0\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on submit with sent\n\
         \x20           add 1 to n\n",
    );
    assert!(
        found.iter().any(|m| m.contains("carries nothing")),
        "{found:?}"
    );
}

/// §18.1's integrity direction, at the site this feature creates: a value
/// the browser chose, written where the program said no browser may
/// choose. Compiling it is the acceptance criterion, and it must fail.
#[test]
fn an_event_payload_may_not_reach_a_trusted_place() {
    let found = refusals(
        "trusted state role is durable Text starting \"guest\"\n\
         state typed is client Text starting \"\"\n\
         view\n\
         \x20   Input typed\n\
         \x20       on keydown with stroke\n\
         \x20           set role to stroke.key\n",
    );
    assert!(
        found.iter().any(|m| m.contains("E-INT-03")),
        "expected the integrity refusal: {found:?}"
    );
    assert!(
        found
            .iter()
            .any(|m| m.contains("stroke") && m.contains("keydown")),
        "the diagnostic must name the payload, not merely the write: {found:?}"
    );
}

/// And the same write of a value the browser did not choose is refused
/// too — because a client-rooted write to `trusted` state is a command,
/// and §18.1 semantics 4 makes every command argument untrusted. Asserted
/// rather than assumed, so the reason in the message is checked to be the
/// right one.
#[test]
fn a_literal_written_to_a_trusted_place_from_a_handler_is_refused_for_its_own_reason() {
    let found = refusals(
        "trusted state role is durable Text starting \"guest\"\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           set role to \"admin\"\n",
    );
    assert!(
        found
            .iter()
            .any(|m| m.contains("a browser sends this write")),
        "{found:?}"
    );
}
