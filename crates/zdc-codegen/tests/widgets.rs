//! `widgets/`, evaluated in the embedded engine.
//!
//! The widget library is ordinary ZDeceptron, so `zdc check` says only that
//! it typechecks. What each widget claims is about the tree after mount and
//! after a keystroke, and three of those claims are the reason the library
//! exists rather than being a naming convention:
//!
//!   1. **The live region is in the document before it has a message.** A
//!      `role="status"` that is created already holding its text has not
//!      changed, and a screen reader announces nothing. The obvious
//!      hand-rolled spelling puts the `if` *around* the region and is
//!      silent; `widgets/toast.zd` puts it inside. Only the rendered tree
//!      can tell the two apart — they typecheck identically.
//!
//!   2. **Escape clears the search field.** The one keyboard behaviour in
//!      the library that is the program's rather than the browser's,
//!      because it is the one that needs no focus to move.
//!
//!   3. **Selecting a tab swaps the panel**, and swaps it for the panel
//!      belonging to that index rather than for whichever one was written
//!      last.
//!
//! Driven the way `component_parity.rs` drives `examples/disclosure.zd`:
//! the DOM shim, the shipped runtime, the emitted module, and a script that
//! fires real events at the rendered tree. `build_example` rather than
//! `compile_example`, because `dashboard.zd` has eight `use` clauses and
//! the single-source path would lose every one of them.

mod support;

use support::{build_example, context, run};

/// Filter the list, clear it with Escape, raise a toast, change tab.
///
/// The order matters in one place: the first frame is captured before
/// anything is fired, because claim 1 is about what is in the document when
/// there is nothing to say.
const DRIVER: &str = r#"
const $host = document.createElement('div');
main($host);
const $frames = [html($host)];

const $buttons = () => walk($host).filter((n) => n.tagName === 'button');
const $labelled = (word) => $buttons().filter((n) => html(n).includes(`>${word}<`));
const $field = walk($host).find((n) => n.tagName === 'input');

// Type a filter that matches one issue of the six.
$field.value = 'Escape';
$field.fire('input');
$frames.push(html($host));

// Clear it from the keyboard, without anything moving focus.
$field.fire('keydown', { key: 'Escape' });
$frames.push(html($host));

// Raise a toast into the live region that was already there.
$labelled('Copy link')[0].fire('click');
$frames.push(html($host));

// Change tab.
$labelled('Activity')[0].fire('click');
$frames.push(html($host));
"#;

fn frames() -> Vec<String> {
    let bundle = build_example("widgets/dashboard.zd");
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        &format!("{DRIVER}\n$frames.join('\\n')"),
    )
    .lines()
    .map(str::to_string)
    .collect()
}

/// Claim 1, in the frame before anything has happened.
///
/// Both halves are asserted, because either alone passes for the broken
/// arrangement: a page with no toast at all also has no message in it.
#[test]
fn the_live_region_exists_before_it_has_a_message() {
    let frames = frames();

    assert!(
        frames[0].contains(r#"role="status""#),
        "the live region must be in the document at mount:\n{}",
        frames[0]
    );
    assert!(
        !frames[0].contains("Dismiss"),
        "and it must be empty until there is something to say:\n{}",
        frames[0]
    );

    // The message arrives into that same region afterwards, which is the
    // change a screen reader announces.
    assert!(
        frames[3].contains("copied."),
        "clicking must put the message in the region:\n{}",
        frames[3]
    );
    assert!(
        frames[3].contains("Dismiss"),
        "and the dismiss control must appear with it:\n{}",
        frames[3]
    );
}

/// Claim 2. The filter narrows, then Escape puts it back.
///
/// `.value` is asserted through the second frame's own contents rather than
/// through the input, because the point is that the *signal* was written:
/// the list is derived from `query`, so a field cleared without the signal
/// changing would leave the list narrowed.
#[test]
fn escape_clears_the_search_field() {
    let frames = frames();

    let unfiltered = frames[0].matches("Copy link").count();
    let filtered = frames[1].matches("Copy link").count();
    let restored = frames[2].matches("Copy link").count();

    assert!(
        unfiltered > filtered,
        "typing a filter must remove issues: {unfiltered} then {filtered}"
    );
    assert_eq!(
        filtered, 1,
        "the filter matches exactly one issue:\n{}",
        frames[1]
    );
    assert_eq!(
        restored, unfiltered,
        "Escape must restore the whole list, which only writing `query` does:\n{}",
        frames[2]
    );
}

/// Claim 3. The panel belonging to the chosen index replaces the one that
/// was open, and nothing else changes place.
#[test]
fn choosing_a_tab_swaps_the_panel() {
    let frames = frames();

    assert!(
        frames[0].contains("Filter issues"),
        "the first panel is open at mount:\n{}",
        frames[0]
    );
    assert!(
        !frames[0].contains("Last week"),
        "and the second panel is not:\n{}",
        frames[0]
    );

    assert!(
        frames[4].contains("Last week"),
        "choosing Activity must open the second panel:\n{}",
        frames[4]
    );
    assert!(
        !frames[4].contains("Filter issues"),
        "and close the first:\n{}",
        frames[4]
    );
    assert!(
        !frames[4].contains("Nothing to configure"),
        "without opening the third:\n{}",
        frames[4]
    );

    // The strip itself survives, so the swap is the panels and not the
    // whole widget being rebuilt.
    assert!(
        frames[4].contains(">Issues<"),
        "the tab strip stays put:\n{}",
        frames[4]
    );
}
