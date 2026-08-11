//! `on key "…"`, evaluated in the embedded engine — §16.3.7a.
//!
//! What a compiler verdict cannot settle is what the emitted bytes do, and
//! every part of this feature is emitted bytes: which module is linked,
//! where the `onKey` call lands relative to the branch that owns it, and
//! whether a keystroke aimed at a field reaches a handler. So the module is
//! run, the same way `event_payloads.rs` runs one, and keys are fired at
//! the document the shim provides.

mod support;

use support::{compile_example, compile_source, context, refusals, run};

/// Open the dialog, press keys at the document, close it, press again.
///
/// The last two frames are the cleanup claim: after the dialog closes, the
/// `Escape` listener that was registered inside the branch must be gone
/// from the document, and the shim's `listenerCount` is what says so.
const DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
const $frames = [];

document.fire('keydown', { key: 'ArrowRight' });
document.fire('keydown', { key: 'ArrowRight' });
document.fire('keydown', { key: 'ArrowLeft' });
document.fire('keydown', { key: 'r' });
$frames.push(html($host));

// The same keystrokes aimed at the field. `rescues` must not move.
const $input = walk($host).find((n) => n.tagName === 'input');
document.fire('keydown', { key: 'r', target: $input });
document.fire('keydown', { key: 'r', target: $input });
$frames.push(html($host));

const $open = walk($host).find((n) => n.tagName === 'button');
$frames.push('listeners before opening: ' + document.listenerCount('keydown'));
$open.fire('click');
$frames.push('listeners while open: ' + document.listenerCount('keydown'));
$frames.push(html($host));

document.fire('keydown', { key: 'Escape' });
$frames.push('listeners after Escape: ' + document.listenerCount('keydown'));
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

/// A named key and a single character both reach a handler, and the
/// handler runs the block it was written with.
#[test]
fn a_document_key_reaches_its_handler() {
    let frames = frames(&compile_example("examples/keys.zd").client_js);
    assert!(
        frames[0].contains("<span>cursor: </span><span>1</span>"),
        "two rights and a left is one: {}",
        frames[0]
    );
    assert!(
        frames[0].contains("<span>rescues: </span><span>1</span>"),
        "`r` is a key too: {}",
        frames[0]
    );
}

/// **The capability rule, in the emitted program.**
///
/// A document listener receives keystrokes aimed at every element on the
/// page. Nothing in `examples/keys.zd` asks for the field to be excluded —
/// the compiler emits it — so this is the test that it is emitted and not
/// merely documented.
#[test]
fn a_keystroke_aimed_at_a_field_does_not_reach_a_document_handler() {
    let frames = frames(&compile_example("examples/keys.zd").client_js);
    assert!(
        frames[1].contains("<span>rescues: </span><span>1</span>"),
        "two `r`s typed into the field moved the counter: {}",
        frames[1]
    );
}

/// **The cleanup claim, counted rather than asserted.**
///
/// The `Escape` handler is written inside `if open`, so it must not exist
/// before the dialog opens, must exist while it is open, and must be gone
/// once the dialog it belongs to has closed. A listener that outlived its
/// branch would keep firing into a graph nothing renders.
#[test]
fn a_listener_written_in_a_branch_is_removed_with_the_branch() {
    let frames = frames(&compile_example("examples/keys.zd").client_js);
    let count = |line: &str| -> usize {
        line.rsplit(' ')
            .next()
            .expect("a trailing number")
            .parse()
            .expect("a number")
    };

    let before = count(&frames[2]);
    let during = count(&frames[3]);
    let after = count(&frames[5]);
    assert_eq!(
        during,
        before + 1,
        "opening the dialog registers exactly one listener: {frames:?}"
    );
    assert_eq!(
        after, before,
        "closing it must remove that listener, not leave it inert: {frames:?}"
    );
    // The other half of the same claim: the listener that was removed is
    // the one that ran. Without this, the counts above are satisfied by a
    // handler that never did anything.
    assert!(
        frames[4].contains("press Escape to close"),
        "the dialog must be on the page while it is open: {}",
        frames[4]
    );
    assert!(
        !frames[6].contains("press Escape to close"),
        "the `Escape` handler inside the branch must have closed it: {}",
        frames[6]
    );
}

/// A program with no `on key` links no `keys.js`, and one with an `on key`
/// links it. Both halves, because the first alone is satisfied by a module
/// nothing ever links.
#[test]
fn keys_js_is_linked_only_by_a_program_that_uses_it() {
    let without = compile_source(
        "state n is client Whole starting 0

view
    Column
        Text n
",
    );
    assert!(
        !without.runtime.contains("runtime/keys.js"),
        "a program with no `on key` shipped the key module: {:?}",
        without.runtime
    );
    assert!(
        !without.client_js.contains("keys.js"),
        "and it imported it: {}",
        without.client_js
    );

    let with = compile_source(
        "state n is client Whole starting 0

view
    Column
        Text n
    on key \"Escape\"
        add 1 to n
",
    );
    assert!(
        with.runtime.contains("runtime/keys.js"),
        "a program with an `on key` must ship the module that drives it: {:?}",
        with.runtime
    );
    assert!(
        with.client_js.contains("/keys.js"),
        "the import list and the shipped set are one decision: {}",
        with.client_js
    );
    // `keys.js` imports `signal.js` and nothing else, so a shortcut does
    // not drag the renderer in beyond what the view already needed.
    assert!(
        !with.runtime.contains("runtime/list.js"),
        "it pulled in the reconciler: {:?}",
        with.runtime
    );
}

/// `on key` under an element is refused, because it does not listen to
/// that element and reading it as if it did is the mistake.
#[test]
fn a_document_key_handler_under_an_element_is_refused() {
    let messages = refusals(
        "state n is client Whole starting 0

view
    Column
        Text n
        on key \"Escape\"
            add 1 to n
",
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("listens to the document, not to `Column`")),
        "got {messages:?}"
    );
}

/// A key the browser never reports is refused where it is written, with
/// the spelling it meant.
///
/// The failure this catches is silence: `on key "Esc"` compiles to a
/// listener that compares against a string no `KeyboardEvent.key` ever
/// equals, and a listener that never fires looks exactly like a key nobody
/// pressed.
#[test]
fn a_key_the_browser_never_reports_is_refused() {
    let messages = refusals(
        "state n is client Whole starting 0

view
    Column
        Text n
    on key \"Esc\"
        add 1 to n
",
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("`Esc` is not a key the browser reports")),
        "got {messages:?}"
    );

    let two = refusals(
        "state n is client Whole starting 0

view
    Column
        Text n
    on key \"gg\"
        add 1 to n
",
    );
    assert!(
        two.iter()
            .any(|m| m.contains("not a key the browser reports")),
        "two characters is not a key: {two:?}"
    );
}
