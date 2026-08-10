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
//!   4. **The tab strip says which tab is chosen, and says of the others
//!      that they are not.** This is the claim the library could not make
//!      at all until `aria-selected` was spellable, and it is the one a
//!      shape check cannot reach: a tablist where the chosen tab carries
//!      `aria-selected="true"` and the rest carry nothing is announced as
//!      a tablist with nothing chosen, and it renders identically.
//!
//!   5. **A switch announces the state it holds, and keeps announcing it
//!      after it changes.** `widgets/toggle.zd` binds `aria-checked` to a
//!      signal, so this is the bound path rather than a constant: it is
//!      the same failure mode as claim 4 with a getter in front of it.
//!
//! Driven the way `component_parity.rs` drives `examples/disclosure.zd`:
//! the DOM shim, the shipped runtime, the emitted module, and a script that
//! fires real events at the rendered tree. `build_example` rather than
//! `compile_example`, because `dashboard.zd` has nine `use` clauses and
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

// Open the panel holding the two controls that are a button plus one
// ARIA state, then operate one of them.
$labelled('Settings')[0].fire('click');
$frames.push(html($host));

$labelled('Notify me about new issues')[0].fire('click');
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
        !frames[4].contains("Notify me about new issues"),
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

/// Claim 4. The strip says which tab is chosen, **and says of the others
/// that they are not**.
///
/// The second half is the whole of it. `aria-selected="true"` on the open
/// tab and no attribute at all on the rest renders identically, passes
/// every shape check, and is announced as a tablist with nothing chosen —
/// which is exactly why `widgets/tabs.zd` wrote no `role="tab"` until the
/// pair could be completed. The panel's name comes back the other way, by
/// `aria-labelledby`, so neither reference is left dangling: the open tab
/// points at a panel that exists and the closed ones point at nothing.
#[test]
fn the_tab_strip_announces_which_tab_is_chosen() {
    let frames = frames();

    for (frame, what) in [(&frames[0], "at mount"), (&frames[4], "after a change")] {
        assert_eq!(
            frame.matches(r#"aria-selected="true""#).count(),
            1,
            "exactly one tab must be chosen {what}:\n{frame}"
        );
        assert_eq!(
            frame.matches(r#"aria-selected="false""#).count(),
            2,
            "and the other two must SAY they are not {what}:\n{frame}"
        );
        assert!(
            frame.contains(r#"role="tablist""#) && frame.contains(r#"role="tabpanel""#),
            "the strip and the open panel must be announced as what they are {what}:\n{frame}"
        );
    }

    // The pair of references, in the direction each is safe. `Activity` is
    // the second tab, so the ids are the ones derived from index 1.
    assert!(
        frames[4].contains(r#"aria-controls="section-panel-1""#),
        "the open tab must name the panel it opened:\n{}",
        frames[4]
    );
    assert!(
        frames[4].contains(r#"id="section-panel-1" aria-labelledby="section-tab-1""#),
        "and the panel must be named by that tab:\n{}",
        frames[4]
    );
    // A closed tab's panel is not in the document, so a reference to it
    // would be one a reader is invited to follow to nothing.
    assert!(
        !frames[4].contains(r#"aria-controls="section-panel-0""#),
        "a closed tab must not point at a panel that was removed:\n{}",
        frames[4]
    );
}

/// Claim 5. A switch announces the state it holds, and goes on announcing
/// it after the state changes.
///
/// The bound path, which is where an ARIA state is easiest to get wrong:
/// `setAttribute` removes an attribute set to `false`, so a switch bound
/// to a signal that is off would carry no `aria-checked` and be announced
/// as an ordinary button. Both frames are asserted, because either alone
/// passes for the broken arrangement — a switch that never renders the
/// attribute has no `true` in the first frame either.
#[test]
fn a_switch_announces_the_state_it_holds_and_then_the_one_it_changes_to() {
    let frames = frames();

    // The whole element, because the claim is that the role and the state
    // are on one control: a `role="switch"` somewhere and an
    // `aria-checked` somewhere else is the arrangement that reads as a
    // switch with no state.
    let switch = |state: &str| {
        format!(r#"role="switch" class="switch" aria-checked="{state}">Notify me about new issues"#)
    };
    assert!(
        frames[5].contains(&switch("false")),
        "a switch that is off must say so rather than saying nothing:\n{}",
        frames[5]
    );
    assert!(
        frames[6].contains(&switch("true")),
        "and pressing it must move the state the reader is told about:\n{}",
        frames[6]
    );
    // A toggle button is a button that stays down, not a setting, so it
    // is `aria-pressed` and carries no role of its own.
    assert!(
        frames[5].contains(r#"aria-pressed="false""#),
        "the toggle button must announce that it is up:\n{}",
        frames[5]
    );
}

/// The three landmarks and the field, named and described.
///
/// Each of these was reachable only by a lower-priority route or not at
/// all: the two `Navigation`s were named by `title`, which also drew a
/// tooltip; the current step of the trail and the current page number were
/// distinguished by *not being links*, which is an inference a reader
/// looking at the page makes instantly and a reader listening to it cannot
/// make; and the one keyboard behaviour in the library was invisible.
#[test]
fn the_landmarks_are_named_and_the_current_position_is_announced() {
    let frames = frames();

    for expected in [
        r#"aria-label="Breadcrumb""#,
        r#"aria-label="Pagination""#,
        // The last crumb and the current page number. Two of them, one per
        // widget, and neither is a link.
        r#"aria-current="page""#,
        // The field says what Escape does, to the reader who cannot see it.
        r#"aria-describedby="issue-filter-note""#,
        r#"id="issue-filter-note""#,
    ] {
        assert!(
            frames[0].contains(expected),
            "`{expected}` is missing from the first frame:\n{}",
            frames[0]
        );
    }
    assert_eq!(
        frames[0].matches(r#"aria-current="page""#).count(),
        2,
        "the trail and the pagination bar each mark one current position:\n{}",
        frames[0]
    );
    // `title` on a landmark was the old route and drew a tooltip nobody
    // asked for; nothing in the library should still be taking it.
    assert!(
        !frames[0].contains(r#"<nav title="#),
        "a landmark must be named by `aria-label` rather than by `title`:\n{}",
        frames[0]
    );
}

/// The first page has a Previous that is present and announced
/// unavailable, rather than a Previous that is not there.
///
/// `widgets/pagination.zd` used to render no button at all, because the
/// only alternative was a live one that silently did nothing. The third
/// answer is `aria-disabled`: in the document, in the tab order, and
/// announced — with no `on click` under it, so it is inert because it has
/// no behaviour rather than because something suppressed one.
#[test]
fn previous_on_the_first_page_is_announced_rather_than_absent() {
    let frames = frames();

    assert!(
        frames[0].contains(r#"aria-disabled="true""#),
        "the first page must still offer a Previous, announced unavailable:\n{}",
        frames[0]
    );
    assert!(
        frames[0].matches(">Previous<").count() == 1,
        "and exactly one of them:\n{}",
        frames[0]
    );
}
