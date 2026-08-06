//! What the elements added to the vocabulary render, asserted against the
//! **parsed DOM** rather than against the emitted string.
//!
//! `element_parity.rs` already compares each built-in's template against
//! the tree `elements.js` builds, which pins the tag, the attributes and
//! the base class. That is a shape check and it is deliberately blind to
//! everything a program does with the element afterwards. This file is the
//! other half: a view is compiled, mounted in the engine, driven, and the
//! resulting tree is read back.

mod support;

use support::{compile_source, context, run};

/// Mount one view and serialise the tree it produced.
fn rendered(source: &str) -> String {
    mounted(&compile_source(source))
}

fn mounted(bundle: &zdc_codegen::Bundle) -> String {
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    )
}

/// The same, for a program whose `static` state the build host has to
/// compute: `build markdown` runs in the compiler's own sandbox, so the
/// values it produces cannot be written down by the test.
fn rendered_after_a_build(source: &str) -> String {
    let module = support::build_module_of(source, "test.zd")
        .expect("this program declares `static` state, so it has a build root");
    let evaluated = zdc_codegen::evaluate(&module, support::repository_path("examples").as_path())
        .unwrap_or_else(|error| panic!("the build root did not run: {}", error.report()));
    let bundle = support::try_compile_with_statics(source, "test.zd", evaluated.values)
        .unwrap_or_else(|errors| panic!("test.zd: {}", errors[0].message));
    mounted(&bundle)
}

/// Fine print is its own element, not a styled span (#58).
#[test]
fn fine_print_renders_as_a_small_element() {
    let tree = rendered("view\n    Small \"terms apply\"\n");
    assert!(
        tree.contains("<small>terms apply</small>"),
        "fine print must carry its own semantics:\n{tree}"
    );
    assert!(
        !tree.contains("<span>terms apply</span>"),
        "a `Small` must not be emitted as a styled span:\n{tree}"
    );
}

/// A matched run of text is a `mark`, which is what a search result
/// highlights (#59). The term comes from a signal, because the whole
/// point of a mark is that what matched is not known when the page is
/// written.
#[test]
fn a_match_renders_as_a_mark_that_tracks_its_signal() {
    let tree = rendered(
        "state term is client Text starting \"parser\"\n\
         view\n\
         \x20   Paragraph \"write the\"\n\
         \x20       Mark term\n",
    );
    assert!(
        tree.contains("<mark>parser</mark>"),
        "a highlighted match must be a mark:\n{tree}"
    );
}

/// An abbreviation carries its expansion, and it carries it where both a
/// pointer and assistive technology look for it (#60).
#[test]
fn an_abbreviation_carries_its_expansion() {
    let tree =
        rendered("view\n    Abbreviation \"HTML\", expansion is \"HyperText Markup Language\"\n");
    assert!(
        tree.contains("<abbr title=\"HyperText Markup Language\">HTML</abbr>"),
        "the expansion must reach `title`:\n{tree}"
    );
}

/// The expansion is the whole reason the element exists, so an
/// abbreviation without one is refused rather than rendered as an
/// unexplained acronym. This follows `Image`'s `alt`.
#[test]
fn an_abbreviation_without_an_expansion_is_refused() {
    let refusals = support::refusals("view\n    Abbreviation \"HTML\"\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Abbreviation` needs `expansion is")),
        "an abbreviation with nothing to expand to must be refused: {refusals:?}"
    );
}

/// Ordinals, chemical formulae and footnote markers, written inline (#61).
#[test]
fn text_can_be_raised_and_lowered() {
    let tree = rendered(
        "view\n\
         \x20   Paragraph \"H\"\n\
         \x20       Subscript \"2\"\n\
         \x20       Text \"O, and the 1\"\n\
         \x20       Superscript \"st\"\n",
    );
    assert!(tree.contains("<sub>2</sub>"), "a subscript:\n{tree}");
    assert!(tree.contains("<sup>st</sup>"), "a superscript:\n{tree}");
}

/// The markdown renderer produces the footnote marker, so a post gets one
/// without the author writing a `Superscript` by hand (#61).
///
/// The marker is a `sup` holding a link to the note, which is what makes
/// it reachable from the keyboard as well as visible. Asserted against the
/// mounted tree: an escaped `&lt;sup&gt;` and a real one are the same
/// string and different documents.
#[test]
fn build_markdown_renders_a_footnote_marker() {
    let tree = rendered_after_a_build(
        "state body is static Markup from render with source is \"\"\"\n\
         \x20   A claim.[^why]\n\
         \n\
         \x20   [^why]: because.\n\
         \x20   \"\"\"\n\
         function render with source\n\
         \x20   give build markdown source\n\
         view\n\
         \x20   Prose body\n",
    );
    assert!(
        tree.contains("<sup"),
        "a footnote marker must be a superscript element:\n{tree}"
    );
    assert!(
        tree.contains("href=\"#why\""),
        "the marker must link to the note:\n{tree}"
    );
    assert!(
        tree.contains("because."),
        "the note itself must render:\n{tree}"
    );
}

/// Contact information is an `address`, which is the semantic a portfolio's
/// contact section has been faking with a `Column` (#62).
#[test]
fn contact_information_renders_as_an_address() {
    let tree = rendered(
        "view\n\
         \x20   Address\n\
         \x20       Link \"mailto:ada@example.com\"\n\
         \x20           Text \"ada@example.com\"\n",
    );
    assert!(
        tree.contains("<address>"),
        "contact information must be an address:\n{tree}"
    );
    assert!(
        tree.contains("href=\"mailto:ada@example.com\""),
        "the contact link must survive the URL sink:\n{tree}"
    );
}

/// A hard break inside a paragraph, which nothing else could produce (#63).
#[test]
fn a_break_ends_a_line_inside_a_paragraph() {
    let tree = rendered(
        "view\n\
         \x20   Paragraph \"Ada Lovelace\"\n\
         \x20       Break\n\
         \x20       Text \"London\"\n",
    );
    let br = tree.find("<br>").unwrap_or_else(|| {
        panic!("a hard break must be a `br`:\n{tree}");
    });
    let first = tree
        .find("Ada Lovelace")
        .expect("the paragraph's own text is on the page");
    let second = tree
        .find("<span>London</span>")
        .unwrap_or_else(|| panic!("the second line is on the page:\n{tree}"));
    assert!(
        first < br && br < second,
        "the break must sit between the two lines:\n{tree}"
    );
}

/// Preserved whitespace that is not code, and the second route to a line
/// break: a block text literal carries one, so the two halves #63 asked
/// for are both reachable (#63).
#[test]
fn preformatted_text_keeps_its_line_breaks_and_is_not_a_code_block() {
    let tree = rendered(
        "view\n\
         \x20   Preformatted \"\"\"\n\
         \x20       one\n\
         \x20       two\n\
         \x20       \"\"\"\n",
    );
    assert!(
        tree.contains("<pre class=\"zd-pre\">one\ntwo</pre>"),
        "preformatted text must keep the line break and say it is not code:\n{tree}"
    );
}

/// A control's accessible name, by an association a browser can follow
/// (#56). The name is the label's own text and the association is `for`
/// against the control's `id`, which is the pairing assistive technology
/// reads.
#[test]
fn a_label_names_the_control_it_points_at() {
    let tree = rendered(
        "state email is client Text starting \"\"\n\
         view\n\
         \x20   Column\n\
         \x20       Label \"Email\", controls is \"email-field\"\n\
         \x20       Input email, id is \"email-field\"\n",
    );
    assert!(
        tree.contains("<label for=\"email-field\">Email</label>"),
        "the label must point at the control by id:\n{tree}"
    );
    assert!(
        tree.contains("id=\"email-field\""),
        "the control must carry the id the label names:\n{tree}"
    );
}

/// A video renders with controls, and its source goes through the same
/// URL sink `Image`'s does (#49).
#[test]
fn a_video_renders_with_controls_and_a_filtered_source() {
    let tree = rendered(
        "view\n    Video source is \"/demo.mp4\", poster is \"/still.png\", width is 640\n",
    );
    assert!(
        tree.contains("<video") && tree.contains("controls"),
        "a media element must be operable:\n{tree}"
    );
    assert!(
        tree.contains("src=\"/demo.mp4\"") && tree.contains("poster=\"/still.png\""),
        "both URLs must reach the DOM:\n{tree}"
    );
    assert!(
        tree.contains("width=\"640\""),
        "a video reserves its box through the attribute, as an image does:\n{tree}"
    );
}

/// Both of a video's URLs are URL-bearing attributes, so a scheme that
/// runs script is refused where it is written rather than filtered at run
/// time (#49).
#[test]
fn a_video_may_not_point_at_a_script_url() {
    let mut checked = 0;
    for source in [
        "view\n    Video source is \"javascript:alert(1)\"\n",
        "view\n    Video source is \"/demo.mp4\", poster is \"javascript:alert(1)\"\n",
    ] {
        checked += 1;
        let refusals = support::refusals(source);
        assert!(
            !refusals.is_empty(),
            "a script URL reached a media element:\n{source}"
        );
    }
    assert_eq!(checked, 2, "both URL-bearing arguments");
}

/// A source is what a video is, so one without it is refused rather than
/// rendered as an empty box.
#[test]
fn a_video_without_a_source_is_refused() {
    let refusals = support::refusals("view\n    Video\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Video` needs `source is")),
        "a video with nothing to play must be refused: {refusals:?}"
    );
}

/// Audio renders with controls, and its source is checked by the same
/// sink (#50).
#[test]
fn audio_renders_with_controls_and_a_filtered_source() {
    let tree = rendered("view\n    Audio source is \"/talk.mp3\"\n");
    assert!(
        tree.contains("<audio") && tree.contains("controls"),
        "a media element must be operable:\n{tree}"
    );
    assert!(
        tree.contains("src=\"/talk.mp3\""),
        "the source must reach the DOM:\n{tree}"
    );

    let refusals = support::refusals("view\n    Audio source is \"javascript:alert(1)\"\n");
    assert!(
        !refusals.is_empty(),
        "a script URL reached an audio element: {refusals:?}"
    );
}

/// The field masks its value, tells the password manager what it is, and
/// keeps it out of the spell checker (#46).
#[test]
fn a_password_field_masks_and_is_not_spell_checked() {
    let tree = rendered(
        "state secretWord is client Text starting \"\"\n\
         view\n\
         \x20   PasswordInput secretWord\n",
    );
    assert!(
        tree.contains("type=\"password\""),
        "the value must be masked:\n{tree}"
    );
    assert!(
        tree.contains("autocomplete=\"current-password\""),
        "a password manager must be told what the field is:\n{tree}"
    );
    assert!(
        tree.contains("spellcheck=\"false\""),
        "a password must not reach the spell checker:\n{tree}"
    );
}

/// The secrecy decision, enforced: the signal a `PasswordInput` binds may
/// appear in the view as that field's binding and nowhere else. Each of
/// these is a sink the value must not reach.
#[test]
fn what_a_password_field_binds_cannot_be_shown_or_sent() {
    let echoed = "state secretWord is client Text starting \"\"\n\
                  view\n\
                  \x20   Column\n\
                  \x20       PasswordInput secretWord\n\
                  \x20       Text secretWord\n";
    let concatenated = "state secretWord is client Text starting \"\"\n\
                        view\n\
                        \x20   Column\n\
                        \x20       PasswordInput secretWord\n\
                        \x20       Text \"you typed \" + secretWord\n";
    let fetched = "state secretWord is client Text starting \"\"\n\
                   view\n\
                   \x20   Column\n\
                   \x20       PasswordInput secretWord\n\
                   \x20       Image source is secretWord, alt is \"nothing\"\n";
    let mirrored = "state secretWord is client Text starting \"\"\n\
                    view\n\
                    \x20   Column\n\
                    \x20       PasswordInput secretWord\n\
                    \x20       Input secretWord\n";
    let mut checked = 0;
    for source in [echoed, concatenated, fetched, mirrored] {
        checked += 1;
        let refusals = support::refusals(source);
        assert!(
            refusals
                .iter()
                .any(|message| message.contains("is what a `PasswordInput` binds")),
            "this program puts a password somewhere it must not go:\n{source}\n{refusals:?}"
        );
    }
    assert_eq!(checked, 4, "four sinks, one program each");
}

/// And the field itself is not refused, so the rule above is about where
/// the value goes rather than about the element existing.
#[test]
fn a_password_field_is_allowed_to_bind_the_signal_it_masks() {
    let tree = rendered(
        "state secretWord is client Text starting \"\"\n\
         view\n\
         \x20   Column\n\
         \x20       Label \"Password\", controls is \"pw\"\n\
         \x20       PasswordInput secretWord, id is \"pw\"\n",
    );
    assert!(
        tree.contains("type=\"password\"") && tree.contains("id=\"pw\""),
        "the field must still compile and carry its own id:\n{tree}"
    );
}

/// A paragraph a person writes, bound the way `Input` is (#41).
///
/// The round trip is what matters: a newline typed into the field has to
/// reach the signal and come back out of it, because that is the one thing
/// a single-line `input` cannot carry.
#[test]
fn a_text_area_carries_newlines_through_the_signal_it_binds() {
    let bundle = compile_source(
        "state note is client Text starting \"\"\n\
         view\n\
         \x20   Column\n\
         \x20       TextArea note, hint is \"say more\"\n\
         \x20       Preformatted note\n",
    );
    let mut context = context(false);
    let frames = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         findTag($host, 'textarea').fire('input', { target: { value: 'one\\ntwo' } });\n\
         serialize($host)",
    );
    assert!(
        frames.contains("<textarea"),
        "a multi-line field must be a textarea:\n{frames}"
    );
    assert!(
        frames.contains("placeholder=\"say more\""),
        "the hint must reach `placeholder`:\n{frames}"
    );
    assert!(
        frames.contains("<pre class=\"zd-pre\">one\ntwo</pre>"),
        "the newline must survive the round trip through the signal:\n{frames}"
    );
}

/// Native disclosure: the summary is the control, and the content follows
/// it inside the same element (#52).
#[test]
fn a_disclosure_renders_as_details_with_a_summary() {
    let tree = rendered(
        "view\n\
         \x20   Details\n\
         \x20       Summary \"How this is built\"\n\
         \x20       Paragraph \"One file.\"\n",
    );
    assert!(
        tree.contains("<details><summary>How this is built</summary><p>One file.</p></details>"),
        "the summary must be the first child of the disclosure:\n{tree}"
    );
}

/// `examples/disclosure.zd` is rewritten on the native element, and the
/// point of the rewrite is what is *absent*: the component keeps no state
/// of its own and the emission allocates no signal for it.
#[test]
fn the_disclosure_example_keeps_no_state_of_its_own() {
    let client = support::compile_example("examples/disclosure.zd").client_js;
    assert!(
        client.contains("<details>") && client.contains("<summary>"),
        "the example must render the native element:\n{client}"
    );
    assert!(
        !client.contains("signal(false)"),
        "the panel's `open` signal must be gone:\n{client}"
    );
    assert!(
        !client.contains("ifInto("),
        "the panel's conditional must be gone with it:\n{client}"
    );
    // The two counters still declare one signal each, so the assertions
    // above are about the panel rather than about an emptied example.
    assert_eq!(
        client.matches("signal(0)").count(),
        2,
        "each `Counter` instance still declares its own state:\n{client}"
    );
}

/// A `details` with no `summary` is labelled with whatever word the browser
/// chose, in whatever language it chose it in, so the name is asked for.
#[test]
fn a_disclosure_without_a_summary_is_refused() {
    let refusals = support::refusals("view\n    Details\n        Paragraph \"hidden\"\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Details` begins with `Summary`")),
        "an unlabelled disclosure must be refused: {refusals:?}"
    );
}

/// Related controls are announced as one group, and the group has a name
/// (#57).
#[test]
fn a_fieldset_groups_its_controls_under_a_legend() {
    let tree = rendered(
        "state post is client Truth starting no\n\
         state email is client Truth starting yes\n\
         view\n\
         \x20   Fieldset\n\
         \x20       Legend \"How to reach you\"\n\
         \x20       Checkbox post, label is \"by post\"\n\
         \x20       Checkbox email, label is \"by email\"\n",
    );
    assert!(
        tree.contains("<fieldset><legend>How to reach you</legend>"),
        "the legend must be the group's first child:\n{tree}"
    );
    assert_eq!(
        tree.matches("type=\"checkbox\"").count(),
        2,
        "both controls must be inside the group:\n{tree}"
    );
}

/// A `fieldset` with no `legend` is announced as a group with no name,
/// which is worse than no grouping at all: a screen reader says "group"
/// before every control in it and never says which group.
#[test]
fn a_fieldset_without_a_legend_is_refused() {
    let refusals = support::refusals(
        "state post is client Truth starting no\n\
         view\n\
         \x20   Fieldset\n\
         \x20       Checkbox post, label is \"by post\"\n",
    );
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Fieldset` begins with `Legend`")),
        "an unnamed group must be refused: {refusals:?}"
    );
}

/// A `Legend` outside a `Fieldset` is an orphan the browser renders as
/// ordinary text, so the placement is checked as `Item`'s is.
#[test]
fn a_legend_outside_a_fieldset_is_refused() {
    let refusals = support::refusals("view\n    Column\n        Legend \"nothing\"\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Legend` must be written directly inside")),
        "an orphaned legend must be refused: {refusals:?}"
    );
}

/// A label pointing at nothing names nothing, so the association is
/// required rather than optional.
#[test]
fn a_label_that_points_at_nothing_is_refused() {
    let refusals = support::refusals("view\n    Label \"Email\"\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Label` needs `controls is")),
        "a label with no control must be refused: {refusals:?}"
    );
}
