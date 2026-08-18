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

/// GitHub-flavoured CommonMark, not bare CommonMark.
///
/// Footnotes alone was the whole option set, and the gap shows the moment a
/// real post is rendered: a table renders as pipes and `~~a~~` renders as
/// tildes. `remark-gfm` is what the site this was tested against reaches
/// for, so a document that renders there and not here is the language's
/// problem rather than the author's.
#[test]
fn build_markdown_renders_the_gfm_extensions() {
    let tree = rendered_after_a_build(
        "state body is static Markup from render with source is \"\"\"\n\
         \x20   | a | b |\n\
         \x20   | --- | --- |\n\
         \x20   | 1 | 2 |\n\
         \n\
         \x20   ~~struck~~\n\
         \n\
         \x20   - [x] done\n\
         \x20   \"\"\"\n\
         function render with source\n\
         \x20   give build markdown source\n\
         view\n\
         \x20   Prose body\n",
    );
    assert!(tree.contains("<table"), "a table must be a table:\n{tree}");
    assert!(
        tree.contains("<del"),
        "`~~a~~` must be struck through:\n{tree}"
    );
    assert!(
        tree.contains("checkbox"),
        "a task list item must be a checkbox:\n{tree}"
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

/// An embed is a trust boundary, and the decision about what it may reach
/// is written into the markup rather than inherited from the platform
/// (#51).
#[test]
fn a_frame_is_sandboxed_and_named() {
    let tree = rendered(
        "view\n    Frame source is \"https://example.com/map\", title is \"A map of the office\"\n",
    );
    assert!(
        tree.contains("<iframe"),
        "an embed must be an iframe:\n{tree}"
    );
    // An empty `sandbox` is the maximally restrictive one: no scripts, no
    // forms, no same-origin, no top-level navigation, no popups. The
    // attribute is present and its value is empty, which is asserted as
    // one thing rather than two so that a `sandbox` carrying any token at
    // all fails here.
    let sandbox = tree
        .split_once("sandbox")
        .map(|(_, rest)| rest.starts_with("=\"\"") || rest.starts_with(' '));
    assert_eq!(
        sandbox,
        Some(true),
        "the sandbox must be present and grant nothing:\n{tree}"
    );
    assert!(
        tree.contains("referrerpolicy=\"no-referrer\""),
        "the embedded document must not be told which page embedded it:\n{tree}"
    );
    assert!(
        tree.contains("title=\"A map of the office\""),
        "an embed needs an accessible name:\n{tree}"
    );
    assert!(
        tree.contains("src=\"https://example.com/map\""),
        "the source must reach the DOM:\n{tree}"
    );
}

/// The sandbox is not widenable, so there is no argument that could relax
/// it and no way to reach one by writing an attribute of that name.
#[test]
fn a_frames_sandbox_cannot_be_widened() {
    let refusals = support::refusals(
        "view\n    Frame source is \"/a\", title is \"a\", sandbox is \"allow-scripts\"\n",
    );
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Frame` has no `sandbox` argument")),
        "the sandbox must not be reachable as an argument: {refusals:?}"
    );
}

/// Its source is a URL-bearing attribute, and an embed's is the worst of
/// them: the document it names runs in the reader's browser.
#[test]
fn a_frame_may_not_point_at_a_script_url() {
    let refusals =
        support::refusals("view\n    Frame source is \"javascript:alert(1)\", title is \"a\"\n");
    assert!(
        !refusals.is_empty(),
        "a script URL reached an embed: {refusals:?}"
    );
}

/// An `iframe` with no `title` is announced as "frame" and nothing else,
/// so the name is required as `Image`'s `alt` is.
#[test]
fn a_frame_without_a_name_is_refused() {
    let refusals = support::refusals("view\n    Frame source is \"/a\"\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Frame` needs `title is")),
        "an unnamed embed must be refused: {refusals:?}"
    );
}

/// One of a named group: two radios over one signal, so picking one clears
/// the other and the group is announced as a group (#43).
#[test]
fn two_radios_over_one_signal_are_one_group() {
    let bundle = compile_source(
        "choice Filter\n\
         \x20   All\n\
         \x20   Finished\n\
         state showing is client Filter starting All\n\
         view\n\
         \x20   Fieldset\n\
         \x20       Legend \"Showing\"\n\
         \x20       Radio showing, option is All, label is \"everything\"\n\
         \x20       Radio showing, option is Finished, label is \"what is done\"\n",
    );
    let mut context = context(false);
    let frames = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $radios = walk($host).filter((n) => n.tagName === 'input');\n\
         const $checked = () => $radios.map((r) => String(r.checked)).join(',');\n\
         const $before = $checked() + ' ' + $radios.map((r) => r.attributes.name).join(',');\n\
         $radios[1].fire('change', { target: { value: 'Finished' } });\n\
         $before + '\\u0001' + $checked()",
    );
    let (before, after) = frames.split_once('\u{1}').expect("two frames");
    assert_eq!(
        before, "true,false showing,showing",
        "the starting variant is checked, and both radios share one group name"
    );
    assert_eq!(
        after, "false,true",
        "picking one must clear the other, because they read one signal"
    );

    // The group has a name, which is what `Fieldset` and `Legend` are for,
    // and each radio has its own.
    let tree = mounted(&bundle);
    assert!(
        tree.contains("<fieldset><legend>Showing</legend>"),
        "the group must be announced as one:\n{tree}"
    );
    assert_eq!(
        tree.matches("class=\"zd-row\"").count(),
        2,
        "each radio is wrapped in its own label:\n{tree}"
    );
}

/// A radio with no label is an unlabelled circle, so it is refused.
#[test]
fn a_radio_without_a_label_is_refused() {
    let refusals = support::refusals(
        "choice Filter\n\
         \x20   All\n\
         \x20   Finished\n\
         state showing is client Filter starting All\n\
         view\n\
         \x20   Radio showing, option is All\n",
    );
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Radio` needs `label is")),
        "an unlabelled radio must be refused: {refusals:?}"
    );
}

/// One choice from a fixed set, and the set is the `choice`'s own arms
/// rather than a list the program repeats (#42).
#[test]
fn a_select_offers_every_arm_of_the_choice_it_binds() {
    let bundle = compile_source(
        "choice Filter\n\
         \x20   All\n\
         \x20   Unfinished\n\
         \x20   Finished\n\
         state showing is client Filter starting All\n\
         view\n\
         \x20   Column\n\
         \x20       Select showing, label is \"Showing\"\n\
         \x20       when showing\n\
         \x20           All\n\
         \x20               Text \"everything\"\n\
         \x20           Unfinished\n\
         \x20               Text \"what is left\"\n\
         \x20           Finished\n\
         \x20               Text \"what is done\"\n",
    );
    let mut context = context(false);
    let frames = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $before = serialize($host);\n\
         findTag($host, 'select').fire('change', { target: { value: 'Finished' } });\n\
         $before + '\\u0001' + serialize($host)",
    );
    let (before, after) = frames.split_once('\u{1}').expect("two frames");
    // One option per arm, in declaration order, and nothing in the program
    // wrote them: the `choice` is the list.
    assert!(
        before.contains(
            "<option value=\"All\">All</option>\
             <option value=\"Unfinished\">Unfinished</option>\
             <option value=\"Finished\">Finished</option>"
        ),
        "the options must come from the choice's arms:\n{before}"
    );
    assert!(
        before.contains("<span>everything</span>"),
        "the starting variant must be the one showing:\n{before}"
    );
    assert!(
        after.contains("<span>what is done</span>"),
        "picking an option must set the signal to that variant:\n{after}"
    );
}

/// A variant's name is an identifier and cannot hold a space, so without a
/// label the only text a `Select` could offer was `DirtBike`. The label is
/// what a person reads; the *value* stays the name, because that is what
/// the runtime's `variant` round-trips on the way back.
#[test]
fn a_select_shows_a_variants_label_and_still_sends_its_name() {
    let bundle = compile_source(
        "choice Equipment\n\
         \x20   DirtBike is \"Dirt Bike\"\n\
         \x20   ATV\n\
         \x20   LawnMower is \"Lawn Mower\"\n\
         state machine is client Equipment starting DirtBike\n\
         view\n\
         \x20   Column\n\
         \x20       Select machine\n\
         \x20       when machine\n\
         \x20           DirtBike\n\
         \x20               Text \"two wheels\"\n\
         \x20           ATV\n\
         \x20               Text \"four wheels\"\n\
         \x20           LawnMower\n\
         \x20               Text \"grass\"\n",
    );
    let mut context = context(false);
    let frames = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $before = serialize($host);\n\
         findTag($host, 'select').fire('change', { target: { value: 'LawnMower' } });\n\
         $before + '\\u0001' + serialize($host)",
    );
    let (before, after) = frames.split_once('\u{1}').expect("two frames");
    // The label is the text; the name is the value. An arm with no label
    // shows its name, so `ATV` is unchanged and nothing had to say so.
    assert!(
        before.contains(
            "<option value=\"DirtBike\">Dirt Bike</option>\
             <option value=\"ATV\">ATV</option>\
             <option value=\"LawnMower\">Lawn Mower</option>"
        ),
        "a label must be the option's text and never its value:\n{before}"
    );
    assert!(
        after.contains("<span>grass</span>"),
        "picking a labelled option must still set the variant:\n{after}"
    );
}

/// A label changes what a `Select` shows and nothing else. It is not a
/// second name for the variant: `when` dispatches on the variant, and an
/// arm written with the label does not parse, because a label is a string
/// and an arm is an identifier.
///
/// Asserted against the parser rather than through `support::refusals`,
/// which panics on a parse error instead of returning it — the refusal
/// here happens a stage earlier than that helper can see.
#[test]
fn a_label_is_not_something_a_program_can_match_on() {
    let error = zdc_parser::parse(
        "choice Equipment\n\
         \x20   DirtBike is \"Dirt Bike\"\n\
         \x20   ATV\n\
         state machine is client Equipment starting DirtBike\n\
         view\n\
         \x20   when machine\n\
         \x20       Dirt Bike\n\
         \x20           Text \"two wheels\"\n\
         \x20       ATV\n\
         \x20           Text \"four wheels\"\n",
    )
    .expect_err("an arm named by a label must not parse");
    assert!(
        !error.message.is_empty(),
        "the refusal must say something: {error:?}"
    );

    // And the same program with the *variant* named parses, so the
    // refusal above is about the label and not about the shape of the
    // `when`.
    zdc_parser::parse(
        "choice Equipment\n\
         \x20   DirtBike is \"Dirt Bike\"\n\
         \x20   ATV\n\
         state machine is client Equipment starting DirtBike\n\
         view\n\
         \x20   when machine\n\
         \x20       DirtBike\n\
         \x20           Text \"two wheels\"\n\
         \x20       ATV\n\
         \x20           Text \"four wheels\"\n",
    )
    .expect("the same `when`, dispatching on the variant, must parse");
}

/// A label and a variant name live in different namespaces: one arm's
/// label may be another arm's name without colliding, because nothing
/// ever looks a variant up by what it is shown as.
#[test]
fn a_label_may_repeat_another_arms_name() {
    let tree = rendered(
        "choice Kind\n\
         \x20   Cycle is \"Bicycle\"\n\
         \x20   Bicycle\n\
         state kind is client Kind starting Cycle\n\
         view\n\
         \x20   Select kind\n",
    );
    // `Cycle` is shown as "Bicycle" and `Bicycle` is shown as "Bicycle",
    // and the two are still different options because the value is the
    // name. Nothing had to be renamed for this to compile.
    assert!(
        tree.contains(
            "<option value=\"Cycle\">Bicycle</option>\
             <option value=\"Bicycle\">Bicycle</option>"
        ),
        "a label matching another arm's name is not a collision:\n{tree}"
    );
}

/// Two arms may carry the same label, and the program still tells them
/// apart. This is the reason the label never reaches `value`: a control
/// whose options were keyed by their text would make these two the same
/// option, and the second one would be unreachable.
#[test]
fn two_arms_may_share_a_label_and_remain_distinct() {
    let bundle = compile_source(
        "choice Kind\n\
         \x20   FrontWheel is \"Wheel\"\n\
         \x20   RearWheel is \"Wheel\"\n\
         state kind is client Kind starting FrontWheel\n\
         view\n\
         \x20   Column\n\
         \x20       Select kind\n\
         \x20       when kind\n\
         \x20           FrontWheel\n\
         \x20               Text \"the front one\"\n\
         \x20           RearWheel\n\
         \x20               Text \"the rear one\"\n",
    );
    let mut context = context(false);
    let frames = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $before = serialize($host);\n\
         findTag($host, 'select').fire('change', { target: { value: 'RearWheel' } });\n\
         $before + '\\u0001' + serialize($host)",
    );
    let (before, after) = frames.split_once('\u{1}').expect("two frames");
    assert!(
        before.contains(
            "<option value=\"FrontWheel\">Wheel</option>\
             <option value=\"RearWheel\">Wheel</option>"
        ),
        "a shared label must leave the two values distinct:\n{before}"
    );
    assert!(
        after.contains("<span>the rear one</span>"),
        "the second arm must still be reachable:\n{after}"
    );
}

/// A variant that carries fields is not an option: an option's value is
/// one string, and there is nowhere for a payload to come from.
#[test]
fn a_select_refuses_a_choice_whose_arms_carry_fields() {
    let refusals = support::refusals(
        "choice Status\n\
         \x20   Open\n\
         \x20   Archived with reason is Text\n\
         state status is client Status starting Open\n\
         view\n\
         \x20   Select status\n",
    );
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("carries fields")),
        "a payload-carrying arm must be refused: {refusals:?}"
    );
}

/// A bounded numeric input: dragging it writes a *number* into the signal,
/// not the text of one, and the bounds are declared rather than validated
/// (#44).
#[test]
fn a_slider_writes_a_number_within_declared_bounds() {
    let bundle = compile_source(
        "state level is client Whole starting 40\n\
         view\n\
         \x20   Column\n\
         \x20       Slider level, least is 0, most is 100, step is 5, label is \"Load\"\n\
         \x20       Text level + 1\n",
    );
    let mut context = context(false);
    let tree = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $before = serialize($host);\n\
         findTag($host, 'input').fire('input', { target: { valueAsNumber: 55 } });\n\
         $before + '\\u0001' + serialize($host)",
    );
    let (before, after) = tree.split_once('\u{1}').expect("two frames");
    for expected in [
        "type=\"range\"",
        "min=\"0\"",
        "max=\"100\"",
        "step=\"5\"",
        "aria-label=\"Load\"",
    ] {
        assert!(
            before.contains(expected),
            "a slider is missing `{expected}`:\n{before}"
        );
    }
    // The proof that a number arrived rather than its text: the view adds
    // one to it, and `"55" + 1` would be `551`.
    assert!(
        before.contains("<span>41</span>"),
        "the starting value must be a number:\n{before}"
    );
    assert!(
        after.contains("<span>56</span>"),
        "dragging must write a number, not the text of one:\n{after}"
    );
}

/// A slider binds a number, so binding text is refused rather than
/// silently producing a control that writes the wrong type.
#[test]
fn a_slider_refuses_a_signal_that_is_not_numeric() {
    let refusals = support::refusals(
        "state name is client Text starting \"\"\n\
         view\n\
         \x20   Slider name, least is 0, most is 10\n",
    );
    assert!(
        !refusals.is_empty(),
        "a text signal reached a slider: {refusals:?}"
    );
}

/// The bounds are what makes the control impossible to drag out of range,
/// so they are required rather than defaulted to nothing in particular.
#[test]
fn a_slider_without_bounds_is_refused() {
    let refusals =
        support::refusals("state level is client Whole starting 1\nview\n    Slider level\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Slider` needs `least is")),
        "an unbounded slider must be refused: {refusals:?}"
    );
}

/// Tabular data as a real table, read back by row and by column (#40).
#[test]
fn a_list_of_records_renders_as_a_table_read_back_by_row_and_column() {
    let bundle = compile_source(
        "record Player\n\
         \x20   name is Text\n\
         \x20   score is Whole\n\
         state players is client List of Player starting \
         [(Player with name is \"ada\", score is 12), (Player with name is \"bo\", score is 7)]\n\
         view\n\
         \x20   Table\n\
         \x20       HeaderRow\n\
         \x20           HeaderCell \"Player\"\n\
         \x20           HeaderCell \"Score\"\n\
         \x20       each player in players\n\
         \x20           TableRow\n\
         \x20               Cell player.name\n\
         \x20               Cell player.score\n",
    );
    let mut context = context(false);
    let cells = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         function rows(node, out) {\n\
         \x20 if (node.tagName === 'tr') out.push(node);\n\
         \x20 for (const kid of node.childNodes || []) rows(kid, out);\n\
         \x20 return out;\n\
         }\n\
         const $rows = rows($host, []);\n\
         $rows\n\
         \x20 .map((r) => (r.childNodes || [])\n\
         \x20   .map((c) => c.tagName + ':' + serialize(c).replace(/<[^>]*>/g, ''))\n\
         \x20   .join(','))\n\
         \x20 .join('|')",
    );
    assert_eq!(
        cells, "th:Player,th:Score|td:ada,td:12|td:bo,td:7",
        "the table must read back by row and column"
    );

    // The header cells say which direction they head, which is what a
    // screen reader announces each data cell with.
    let tree = mounted(&bundle);
    assert_eq!(
        tree.matches("scope=\"col\"").count(),
        2,
        "each header cell must declare its scope:\n{tree}"
    );
    // The rows sit inside one row group, written by the compiler rather
    // than by the browser's parser, so the offsets every binding is
    // scheduled against are the ones the template really parses into.
    assert_eq!(
        tree.matches("<tbody>").count(),
        1,
        "one row group, written out:\n{tree}"
    );
}

/// The table family's nesting is checked, because a `td` outside a `tr` is
/// foster-parented out of the table by the browser's own parser and every
/// binding after it would point at the wrong node.
#[test]
fn a_cell_outside_a_row_is_refused() {
    let refusals = support::refusals("view\n    Column\n        Cell \"orphan\"\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Cell` must be written directly inside")),
        "an orphaned cell must be refused: {refusals:?}"
    );
}

/// A submit boundary with one handler: Enter inside a field submits the
/// form once, with every bound value already set (#39).
#[test]
fn enter_inside_a_field_submits_the_form_once() {
    let bundle = compile_source(
        "state name is client Text starting \"\"\n\
         state greeted is client Text starting \"\"\n\
         view\n\
         \x20   Form\n\
         \x20       on submit\n\
         \x20           set greeted to name\n\
         \x20       Input name\n\
         \x20       Button \"send\"\n",
    );
    let mut context = context(false);
    let tree = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         findTag($host, 'input').fire('input', { target: { value: 'Ada' } });\n\
         let $defaulted = true;\n\
         findTag($host, 'form').fire('submit', { preventDefault: () => { $defaulted = false; } });\n\
         serialize($host) + '\\u0001' + $defaulted",
    );
    let (page, defaulted) = tree.split_once('\u{1}').expect("two answers");
    assert!(page.contains("<form>"), "the group must be a form:\n{page}");
    assert_eq!(
        defaulted, "false",
        "submitting must not let the browser navigate away:\n{page}"
    );
    assert!(
        page.contains(".value=\"Ada\""),
        "the field's value must be set when the handler runs:\n{page}"
    );
}

/// A `form` with no submit handler navigates away on Enter and loses every
/// value on the page, so it is refused rather than emitted.
#[test]
fn a_form_without_a_submit_handler_is_refused() {
    let refusals = support::refusals("view\n    Form\n        Button \"send\"\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Form` needs `on submit`")),
        "a form that would navigate away must be refused: {refusals:?}"
    );
}

/// A measurement inside a range, with the landmarks a reader interprets it
/// by (#55). Not a progress bar: this shows where a value sits, not how
/// far a task has got.
#[test]
fn a_meter_shows_a_value_within_a_declared_range() {
    let tree = rendered(
        "state level is client Whole starting 40\n\
         view\n\
         \x20   Meter level, least is 0, most is 100, low is 20, high is 80, best is 60, \
         label is \"Load\"\n",
    );
    for expected in [
        "<meter",
        "min=\"0\"",
        "max=\"100\"",
        "low=\"20\"",
        "high=\"80\"",
        "optimum=\"60\"",
        "aria-label=\"Load\"",
        "value=\"40\"",
    ] {
        assert!(
            tree.contains(expected),
            "a meter is missing `{expected}`:\n{tree}"
        );
    }
}

/// `examples/gauge.zd` shows the same number twice: once through a foreign
/// that owns a canvas, and once through the element the language has.
#[test]
fn the_gauge_example_renders_on_a_real_meter() {
    let client = support::compile_example("examples/gauge.zd").client_js;
    assert!(
        client.contains("<meter") && client.contains("max=\"100\""),
        "the gauge example must render a meter with a declared range:\n{client}"
    );
}

/// Completion toward a goal, announced by the browser (#54).
#[test]
fn a_progress_bar_shows_a_numeric_signal_and_tracks_it() {
    let bundle = compile_source(
        "state done is client Whole starting 3\n\
         view\n\
         \x20   Column\n\
         \x20       Progress done, most is 10, label is \"Upload\"\n\
         \x20       Button \"step\"\n\
         \x20           on click\n\
         \x20               add 1 to done\n",
    );
    let mut context = context(false);
    let frames = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $before = serialize($host);\n\
         findTag($host, 'button').fire('click');\n\
         $before + '\\u0001' + serialize($host)",
    );
    let (before, after) = frames.split_once('\u{1}').expect("two frames");
    assert!(
        before.contains("<progress") && before.contains("max=\"10\""),
        "a progress element with a declared goal:\n{before}"
    );
    assert!(
        before.contains("aria-label=\"Upload\""),
        "the element must be announced by name:\n{before}"
    );
    assert!(
        before.contains("value=\"3\""),
        "the value comes from the signal:\n{before}"
    );
    assert!(after.contains("value=\"4\""), "and it tracks it:\n{after}");
}

/// A progress bar shows a number, so it refuses text rather than rendering
/// a bar at zero.
#[test]
fn a_progress_bar_refuses_a_value_that_is_not_a_number() {
    let refusals = support::refusals("view\n    Progress \"most of the way\"\n");
    assert!(
        !refusals.is_empty(),
        "text reached a progress element: {refusals:?}"
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

/// A typed numeric field yields a **number**, and an empty one yields
/// `None` rather than zero or `NaN` (#45).
///
/// The proof that a number arrived is the same one `Slider`'s test uses:
/// the view adds one to it, and `"41" + 1` would be `411`.
#[test]
fn a_number_input_writes_a_number_and_an_empty_field_writes_none() {
    let bundle = compile_source(
        "state count is client Option of Whole starting None\n\
         view\n\
         \x20   Column\n\
         \x20       NumberInput count, least is 0, most is 99, step is 1, hint is \"how many\"\n\
         \x20       when count\n\
         \x20           None\n\
         \x20               Text \"nothing yet\"\n\
         \x20           Some with here\n\
         \x20               Text here + 1\n",
    );
    let mut context = context(false);
    let tree = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $field = findTag($host, 'input');\n\
         const $empty = serialize($host);\n\
         $field.value = '41'; $field.fire('input');\n\
         const $typed = serialize($host);\n\
         $field.value = ''; $field.fire('input');\n\
         $empty + '\\u0001' + $typed + '\\u0001' + serialize($host)",
    );
    let mut frames = tree.split('\u{1}');
    let (empty, typed, cleared) = (
        frames.next().expect("a frame"),
        frames.next().expect("a frame"),
        frames.next().expect("a frame"),
    );
    for expected in [
        "type=\"number\"",
        "min=\"0\"",
        "max=\"99\"",
        "step=\"1\"",
        "placeholder=\"how many\"",
    ] {
        assert!(
            empty.contains(expected),
            "a number field is missing `{expected}`:\n{empty}"
        );
    }
    assert!(
        empty.contains("<span>nothing yet</span>"),
        "an empty field must be `None` and not a silent zero:\n{empty}"
    );
    assert!(
        typed.contains("<span>42</span>"),
        "typing must write a number, not the text of one:\n{typed}"
    );
    assert!(
        cleared.contains("<span>nothing yet</span>"),
        "clearing the field must go back to `None`:\n{cleared}"
    );
}

/// The program writes the field as well as reading it, so `set count to
/// None` empties the box rather than leaving a stale number in it.
#[test]
fn writing_the_signal_writes_the_number_field() {
    let bundle = compile_source(
        "state count is client Option of Whole starting Some with value is 7\n\
         view\n\
         \x20   Column\n\
         \x20       NumberInput count\n\
         \x20       Button \"clear\"\n\
         \x20           on click\n\
         \x20               set count to None\n",
    );
    let mut context = context(false);
    let tree = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $before = serialize($host);\n\
         findTag($host, 'button').fire('click');\n\
         $before + '\\u0001' + serialize($host)",
    );
    let (before, after) = tree.split_once('\u{1}').expect("two frames");
    assert!(
        before.contains(".value=\"7\""),
        "the starting number must reach the box:\n{before}"
    );
    assert!(
        after.contains(".value=\"\""),
        "`None` must empty the box:\n{after}"
    );
}

/// A number field binds an `Option`, because an empty box holds no
/// number. A bare `Whole` has nowhere to put that, so it is refused
/// rather than silently reading as zero.
#[test]
fn a_number_input_refuses_a_signal_that_cannot_be_empty() {
    let refusals =
        support::refusals("state count is client Whole starting 0\nview\n    NumberInput count\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("binds an `Option of Whole`")),
        "a non-optional signal reached a number field: {refusals:?}"
    );
}

/// A date field binds a **moment** — the `Whole` of milliseconds
/// `prelude/time.zd` reads apart — so the calendar the language already
/// has applies to what a reader picked (#48).
#[test]
fn a_date_input_writes_a_moment_the_prelude_can_read() {
    let bundle = compile_source(
        "state born is client Option of Whole starting None\n\
         view\n\
         \x20   Column\n\
         \x20       DateInput born\n\
         \x20       when born\n\
         \x20           None\n\
         \x20               Text \"no day\"\n\
         \x20           Some with moment\n\
         \x20               Text (civilDateOf of moment).year\n\
         \x20               Text (civilDateOf of moment).month\n\
         \x20               Text (civilDateOf of moment).day\n",
    );
    let mut context = context(false);
    let tree = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $field = findTag($host, 'input');\n\
         const $empty = serialize($host);\n\
         $field.value = '2024-02-29'; $field.fire('input');\n\
         $empty + '\\u0001' + serialize($host)",
    );
    let (empty, picked) = tree.split_once('\u{1}').expect("two frames");
    assert!(
        empty.contains("type=\"date\""),
        "a date field must be a native picker:\n{empty}"
    );
    assert!(
        empty.contains("<span>no day</span>"),
        "an empty picker must be `None`:\n{empty}"
    );
    // The leap day, read apart by the prelude's own civil calendar. If
    // the element yielded anything but a moment, `civilDateOf` could not
    // be applied to it at all.
    for expected in ["<span>2024</span>", "<span>2</span>", "<span>29</span>"] {
        assert!(
            picked.contains(expected),
            "the moment must read apart as the day that was picked, missing `{expected}`:\n\
             {picked}"
        );
    }
}

/// The moment goes back into the picker, which is what makes the binding
/// two-way rather than a read of the control.
#[test]
fn a_moment_written_by_the_program_reaches_the_picker() {
    let bundle = compile_source(
        "state born is client Option of Whole starting None\n\
         view\n\
         \x20   Column\n\
         \x20       DateInput born\n\
         \x20       Button \"pick\"\n\
         \x20           on click\n\
         \x20               set born to Some with value is 1709164800000\n",
    );
    let mut context = context(false);
    let tree = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         findTag($host, 'button').fire('click');\n\
         serialize($host)",
    );
    // 1709164800000 is 2024-02-29T00:00:00Z.
    assert!(
        tree.contains(".value=\"2024-02-29\""),
        "a moment the program wrote must show as the day it names:\n{tree}"
    );
}

/// A moment is a count of milliseconds, so a `Decimal` is refused rather
/// than floored somewhere out of sight.
#[test]
fn a_date_input_refuses_anything_but_a_moment() {
    let refusals = support::refusals(
        "state born is client Option of Decimal starting None\nview\n    DateInput born\n",
    );
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Option of Whole` is expected here")),
        "a fractional moment reached a date field: {refusals:?}"
    );
}

// --- the file picker (#47) -------------------------------------------------

/// What a `FileInput` yields: the **name** of the file a reader chose, as
/// an `Option of Text`, and nothing else about the file.
///
/// The two frames are the whole contract. Before anything is chosen the
/// signal is `None`, which is why the type is an `Option` rather than a
/// `Text` — an empty picker is not a file called nothing. After a choice
/// it is `Some` of the name the file was saved under.
#[test]
fn a_file_input_writes_the_name_of_what_was_chosen() {
    let bundle = compile_source(
        "state chosen is client Option of Text starting None\n\
         view\n\
         \x20   Column\n\
         \x20       FileInput chosen\n\
         \x20       when chosen\n\
         \x20           None\n\
         \x20               Text \"nothing yet\"\n\
         \x20           Some with name\n\
         \x20               Text name\n",
    );
    let mut context = context(false);
    let tree = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $field = findTag($host, 'input');\n\
         const $empty = serialize($host);\n\
         // What the browser puts on the control: a `FileList`, of which\n\
         // the binding reads one field of one entry.\n\
         $field.files = [{ name: 'report.csv', size: 4096, type: 'text/csv' }];\n\
         $field.fire('change');\n\
         $empty + '\\u0001' + serialize($host)",
    );
    let (empty, picked) = tree.split_once('\u{1}').expect("two frames");
    assert!(
        empty.contains("type=\"file\""),
        "a file picker must be the browser's own control:\n{empty}"
    );
    assert!(
        empty.contains("<span>nothing yet</span>"),
        "a picker nobody has used must be `None`:\n{empty}"
    );
    assert!(
        picked.contains("<span>report.csv</span>"),
        "the name of the chosen file must reach the program:\n{picked}"
    );
}

/// `None` empties the control, which is the *only* write the browser
/// permits and the reason the write half exists at all.
///
/// Without it a form that resets itself after an upload would leave the
/// old file named in the picker under a program that believes nothing is
/// chosen — two places one piece of state lives, disagreeing.
#[test]
fn clearing_the_signal_empties_the_picker() {
    let bundle = compile_source(
        "state chosen is client Option of Text starting None\n\
         view\n\
         \x20   Column\n\
         \x20       FileInput chosen\n\
         \x20       Button \"reset\"\n\
         \x20           on click\n\
         \x20               set chosen to None\n",
    );
    let mut context = context(false);
    let tree = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $field = findTag($host, 'input');\n\
         $field.files = [{ name: 'report.csv' }];\n\
         // The fake path every browser reports for a chosen file.\n\
         $field.value = 'C:\\\\fakepath\\\\report.csv';\n\
         $field.fire('change');\n\
         const $held = serialize($host);\n\
         findTag($host, 'button').fire('click');\n\
         $held + '\\u0001' + serialize($host)",
    );
    let (held, cleared) = tree.split_once('\u{1}').expect("two frames");
    assert!(
        held.contains("fakepath"),
        "a chosen file leaves the control non-empty:\n{held}"
    );
    assert!(
        !cleared.contains("fakepath"),
        "writing `None` must empty the control:\n{cleared}"
    );
}

/// A picker binds a name that may be absent, so a bare `Text` is refused
/// rather than given an empty string that means two different things.
#[test]
fn a_file_input_refuses_a_signal_that_cannot_be_empty() {
    let refusals = support::refusals(
        "state chosen is client Text starting \"\"\nview\n    FileInput chosen\n",
    );
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Option of Text` is expected here")),
        "a non-optional signal reached a file picker: {refusals:?}"
    );
}

/// §14B.5, unchanged and unwidened: choosing a file must not silently
/// become a network write.
///
/// **This is the placement rule the issue asked to see written down, and
/// what it shows is that there is no new one.** The element binds `Text`,
/// whose placement rules already exist, so `durable` is refused by the
/// rule every other two-way element is refused by, in the words it
/// already had. A `File`-typed binding would have needed a rule of its
/// own about a value meaningful only in the tab that made it.
#[test]
fn a_file_input_refuses_a_signal_a_keystroke_cannot_write() {
    let refusals = support::refusals(
        "state chosen is durable Option of Text starting None\nview\n    FileInput chosen\n",
    );
    assert!(
        refusals.iter().any(|message| {
            message.contains("`FileInput` writes back into it from the browser")
        }),
        "a `durable` signal reached a file picker: {refusals:?}"
    );
}

/// The picker's own `on change` is taken by the binding, so a second
/// handler for it is refused rather than left to fight the built-in one.
#[test]
fn a_second_change_handler_on_a_file_input_is_refused() {
    let refusals = support::refusals(
        "state chosen is client Option of Text starting None\n\
         view\n\
         \x20   FileInput chosen\n\
         \x20       on change\n\
         \x20           set chosen to None\n",
    );
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("already wires `on change`")),
        "a file picker accepted a second change handler: {refusals:?}"
    );
}

/// Neither field's `on input` may be written twice: the built-in binding
/// already occupies it, and a second handler would fight it.
#[test]
fn a_second_input_handler_on_a_numeric_field_is_refused() {
    // Counted: both elements share one slot, so a list that lost one
    // would still pass the assertions it no longer ran.
    let mut checked = 0;
    for element in ["NumberInput", "DateInput"] {
        checked += 1;
        let refusals = support::refusals(&format!(
            "state count is client Option of Whole starting None\n\
             view\n\
             \x20   {element} count\n\
             \x20       on input\n\
             \x20           set count to None\n"
        ));
        assert!(
            refusals
                .iter()
                .any(|message| message.contains("already wires `on input`")),
            "`{element}` accepted a second input handler: {refusals:?}"
        );
    }
    assert_eq!(checked, 2, "both fields bound to `Slot::OptionalLevel`");
}

// --- the ARIA arguments (§16.3.6, the aria table) --------------------------
//
// `element_parity.rs` pins the two implementations to the same tree for a
// constant argument. What it cannot see is the case the whole feature turns
// on: a state whose value is a *signal*, which is what a tab strip has.

/// **An ARIA state is the word `false`, never the absence of an attribute.**
///
/// This is the one that would have shipped broken. `dom.js`'s
/// `setAttribute` implements HTML's boolean attributes, so a bound `false`
/// removes the attribute; an unselected tab would then carry no
/// `aria-selected` at all, and a tablist in which no tab says `false` is
/// announced as one with nothing chosen. The tree is what tells the two
/// apart — both spellings typecheck, both render, and only one of them
/// says which tab is open.
#[test]
fn a_bound_aria_state_reaches_the_dom_as_a_word_in_both_positions() {
    let bundle = support::compile_source(
        "state chosen is client Whole starting 0\n\
         view\n\
         \x20   Row role is \"tablist\"\n\
         \x20       Button \"Issues\", role is \"tab\", selected is chosen is 0\n\
         \x20           on click\n\
         \x20               set chosen to 0\n\
         \x20       Button \"Activity\", role is \"tab\", selected is chosen is 1\n\
         \x20           on click\n\
         \x20               set chosen to 1\n",
    );
    let mut context = context(false);
    let frames = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $first = serialize($host);\n\
         walk($host).filter((n) => n.tagName === 'button')[1].fire('click');\n\
         $first + '\\n' + serialize($host)",
    );
    let (before, after) = frames.split_once('\n').expect("two frames");

    assert_eq!(
        before.matches(r#"aria-selected="true""#).count(),
        1,
        "exactly one tab is selected at mount:\n{before}"
    );
    assert_eq!(
        before.matches(r#"aria-selected="false""#).count(),
        1,
        "and the other must SAY it is not, rather than saying nothing:\n{before}"
    );

    // The state follows the signal, which is the whole point of binding it.
    assert!(
        after.contains(r#"<button type="button" role="tab" aria-selected="false">Issues</button>"#),
        "the first tab must give the selection up:\n{after}"
    );
    assert!(
        after
            .contains(r#"<button type="button" role="tab" aria-selected="true">Activity</button>"#),
        "and the second must take it:\n{after}"
    );
}

/// The states, one per attribute, so a row lost from the table is a
/// failing test rather than an argument that quietly becomes an attribute
/// of its own name.
#[test]
fn every_aria_state_reaches_the_attribute_it_names() {
    // Counted: the assertion is inside the loop, so an emptied list would
    // pass over nothing.
    let mut checked = 0;
    for (argument, attribute) in [
        ("selected", "aria-selected"),
        ("expanded", "aria-expanded"),
        ("pressed", "aria-pressed"),
        ("checked", "aria-checked"),
        ("disabled", "aria-disabled"),
        ("decorative", "aria-hidden"),
    ] {
        checked += 1;
        let tree = rendered(&format!("view\n    Button \"go\", {argument} is yes\n"));
        assert!(
            tree.contains(&format!(r#"{attribute}="true""#)),
            "`{argument}` must reach `{attribute}`:\n{tree}"
        );
        let off = rendered(&format!("view\n    Button \"go\", {argument} is no\n"));
        assert!(
            off.contains(&format!(r#"{attribute}="false""#)),
            "`{argument} is no` must write the word, not remove the attribute:\n{off}"
        );
    }
    assert_eq!(checked, 6, "every ARIA state must be checked");
}

/// The references and the closed word sets. `label` is here too: on
/// anything with no text beside it to wrap, it is `aria-label`.
#[test]
fn every_aria_reference_and_word_reaches_the_attribute_it_names() {
    let tree = rendered(
        "view\n\
         \x20   Navigation label is \"Breadcrumb\"\n\
         \x20       Text \"Issues\", current is \"page\", live is \"polite\", \
         controls is \"panel\", describedBy is \"note\", labelledBy is \"trail\"\n",
    );
    for expected in [
        r#"aria-label="Breadcrumb""#,
        r#"aria-current="page""#,
        r#"aria-live="polite""#,
        r#"aria-controls="panel""#,
        r#"aria-describedby="note""#,
        r#"aria-labelledby="trail""#,
    ] {
        assert!(tree.contains(expected), "`{expected}` is missing:\n{tree}");
    }
}

/// `controls` is the one argument whose attribute depends on the element,
/// and both meanings are the same sentence: which control this one
/// operates. A `label` has HTML's own `for`, which is what clicking it
/// acts on; nothing else has one.
#[test]
fn controls_is_for_on_a_label_and_aria_controls_everywhere_else() {
    let label = rendered("view\n    Label \"Email\", controls is \"email-field\"\n");
    assert!(
        label.contains(r#"<label for="email-field">"#),
        "a label points at its control with `for`:\n{label}"
    );
    assert!(
        !label.contains("aria-controls"),
        "and must not also claim it with ARIA:\n{label}"
    );

    let button = rendered("view\n    Button \"Issues\", controls is \"panel\"\n");
    assert!(
        button.contains(r#"aria-controls="panel""#),
        "everything else says it with `aria-controls`:\n{button}"
    );
}

/// A `Checkbox` still reads `label` itself and wraps its box in one, which
/// is the split that would break first if `label` became an attribute
/// everywhere.
#[test]
fn a_checkbox_label_still_wraps_the_box_rather_than_becoming_an_attribute() {
    let tree = rendered(
        "state done is client Truth starting no\nview\n    Checkbox done, label is \"ready\"\n",
    );
    assert!(
        tree.contains("<label class=\"zd-row\">"),
        "the box must be wrapped in the label it was given:\n{tree}"
    );
    assert!(
        !tree.contains("aria-label"),
        "and the word must not also be an attribute:\n{tree}"
    );
}

/// A state is a `Truth`. `selected is "yes"` is the mistake worth
/// refusing: it is text that reads as a truth, and `aria-selected="yes"`
/// is a token every browser maps onto `true` — so the wrong spelling
/// announces the right answer for the chosen tab and the right answer
/// again for all the others.
#[test]
fn an_aria_state_given_text_is_refused() {
    let refusals = support::refusals("view\n    Button \"go\", selected is \"yes\"\n");
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`selected` is `Text`")),
        "a text that reads as a truth reached an ARIA state: {refusals:?}"
    );
}

/// A word outside the set is refused, and so is a word that exists only at
/// run time. Neither can be caught later: a browser maps every
/// unrecognised `aria-current` onto `true` rather than ignoring it.
#[test]
fn an_aria_word_must_be_written_down_and_must_be_one_of_the_set() {
    let outside = support::refusals("view\n    Text \"here\", current is \"pge\"\n");
    assert!(
        outside
            .iter()
            .any(|message| message.contains("A `current` is one of `page`, `step`")),
        "a word outside the set was accepted: {outside:?}"
    );

    let computed = support::refusals(
        "state kind is client Text starting \"page\"\nview\n    Text \"here\", current is kind\n",
    );
    assert!(
        computed
            .iter()
            .any(|message| message.contains("`current` must be written down")),
        "a word that exists only at run time was accepted: {computed:?}"
    );
}

/// The `disabled` style prefix and the `disabled` argument are one
/// sentence. `:disabled` alone matched nothing this language can write —
/// there is no `disabled` attribute in the vocabulary — so a control
/// announced unavailable can now also be drawn that way.
#[test]
fn a_disabled_style_reaches_a_control_disabled_the_only_way_this_language_can_be() {
    let bundle = support::compile_source(
        "view\n    Button \"Previous\", disabled is yes, disabledColor is \"grey\"\n",
    );
    assert!(
        bundle
            .styles_css
            .contains(r#":is(:disabled,[aria-disabled="true"]) { color: grey; }"#),
        "the rule must reach the only disabled this language can write:\n{}",
        bundle.styles_css
    );
    assert!(
        mounted(&bundle).contains(r#"aria-disabled="true""#),
        "and the control must be the thing that rule selects"
    );
}

// --- Dialog (#53) ---------------------------------------------------------
//
// The open/closed state, the write-back and the deferred opening are
// driven here. The focus trap, Escape and the return of focus to whatever
// opened the dialog are deliberately not: they are `showModal()`'s own
// behaviour and the shim has no focus, no top layer and no `inert` to ask
// about, so `zdc-cli/tests/browser.rs` asks a real browser instead.
//
// What the shim does model is the state machine, and it models it by
// throwing where a browser throws — `showModal()` refuses an already-open
// dialog and refuses a detached node — so a binding that got either wrong
// fails here rather than quietly doing nothing.
//
// The verdicts are read off `dialog.open` rather than out of the
// serialised tree, because what the binding must get right is the
// element's *state*: a dialog holding an `open` attribute it was given
// rather than one `showModal()` wrote is the exact mistake this element
// exists to make unwritable, and the two look identical in a string.

/// A modal is opened and closed by the signal it binds, and by nothing
/// else (#53).
///
/// The shim's `showModal` throws on a node that is not in the document and
/// on one that is already open, exactly as a browser does, so a binding
/// that called the method twice — or that reached for the attribute
/// instead — fails here rather than rendering something that looks right.
#[test]
fn a_dialog_opens_and_closes_from_the_signal_that_binds_it() {
    let bundle = compile_source(
        "state confirming is client Truth starting no\n\
         view\n\
         \x20   Column\n\
         \x20       Button \"Delete\"\n\
         \x20           on click\n\
         \x20               set confirming to yes\n\
         \x20       Dialog confirming, label is \"Confirm deletion\"\n\
         \x20           Text \"Delete it?\"\n",
    );
    // The template carries no `open`. A dialog whose markup said `open`
    // is one `showModal()` refuses to open — and until it threw it would
    // have been showing with no backdrop, no focus trap and no Escape.
    assert!(
        bundle
            .client_js
            .contains("<dialog aria-label=\"Confirm deletion\">"),
        "the template must be a plain closed dialog:\n{}",
        bundle.client_js
    );

    let mut context = context(false);
    let verdict = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $dialog = findTag($host, 'dialog');\n\
         const $shut = $dialog.open;\n\
         findTag($host, 'button').fire('click');\n\
         'shut=' + $shut + ' shown=' + $dialog.open + \
         ' name=' + $dialog.attributes['aria-label'] + \
         ' holds=' + serialize($host).includes('Delete it?')",
    );
    assert_eq!(
        verdict, "shut=false shown=true name=Confirm deletion holds=true",
        "a dialog must start closed, open when its signal is written, carry the name it was \
         given, and hold its children"
    );
}

/// **Closing writes back, so the button that opened the modal still
/// works.**
///
/// This is the failure the element exists to make unwritable. Escape and
/// the browser's own close request close a `<dialog>` without asking the
/// program; if the signal is not written back it stays `yes`, the effect
/// sees no change on the next click, and the page is dead with nothing
/// reported anywhere. `close()` is the step a close request performs, so
/// calling it is exercising the same path Escape reaches.
///
/// The `if` region is in the view to make the disagreement visible: it
/// reads the same signal, so it is what the *program* believes, while
/// `dialog.open` is what the DOM is doing.
#[test]
fn closing_a_dialog_writes_back_so_the_next_click_reopens_it() {
    let bundle = compile_source(
        "state confirming is client Truth starting no\n\
         view\n\
         \x20   Column\n\
         \x20       Button \"Delete\"\n\
         \x20           on click\n\
         \x20               set confirming to yes\n\
         \x20       if confirming\n\
         \x20           Text \"the program says open\"\n\
         \x20       Dialog confirming, label is \"Confirm deletion\"\n\
         \x20           Text \"Delete it?\"\n",
    );
    let mut context = context(false);
    let verdict = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $dialog = findTag($host, 'dialog');\n\
         const $button = findTag($host, 'button');\n\
         const $believes = () => serialize($host).includes('the program says open');\n\
         $button.fire('click');\n\
         const $shown = $dialog.open + '/' + $believes();\n\
         $dialog.close();\n\
         const $dismissed = $dialog.open + '/' + $believes();\n\
         $button.fire('click');\n\
         'shown=' + $shown + ' dismissed=' + $dismissed + ' again=' + $dialog.open",
    );
    assert_eq!(
        verdict, "shown=true/true dismissed=false/false again=true",
        "a dismissed dialog must stay shut, the program must learn that it did, and the button \
         that opened it must open it again"
    );
}

/// A dialog whose signal starts `yes` opens once it is in the document,
/// rather than throwing at load.
///
/// `showModal()` throws `InvalidStateError` on a node that is not
/// connected. Left alone that is #205's shape exactly: an exception during
/// module evaluation, a body that stays empty, and nothing said anywhere.
/// `elements.js` therefore has two arms — open now if the node is in the
/// document, and defer to the microtask after insertion if it is not.
///
/// **The root mounts its own tree before its bindings run, so this program
/// takes the first arm.** It used to take the second, because a binding
/// ran against a `<template>` clone; the assertion below moved when the
/// mount did, and what it is really checking did not — the shim throws if
/// `showModal()` is ever reached on a disconnected node, so a run that
/// returns at all proves the element never took that path. The deferred
/// arm is still live for a dialog that arrives in a hole filled later.
#[test]
fn a_dialog_that_starts_open_opens_after_the_tree_is_in_the_document() {
    let bundle = compile_source(
        "state showing is client Truth starting yes\n\
         view\n\
         \x20   Dialog showing, label is \"Welcome\"\n\
         \x20       Text \"Hello\"\n",
    );
    let mut context = context(false);
    let verdict = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $dialog = findTag($host, 'dialog');\n\
         const $mounted = $dialog.open;\n\
         flushMicrotasks();\n\
         'mounted=' + $mounted + ' settled=' + $dialog.open",
    );
    assert_eq!(
        verdict, "mounted=true settled=true",
        "the dialog must be open by the time the mounting task ends, and must not have reached \
         `showModal()` on a node outside the document to get there"
    );
}

/// A modal moves focus into itself, so what a reader hears on arrival is
/// its accessible name — and a `dialog` has none of its own. This follows
/// `Image`'s `alt` and `Frame`'s `title`.
#[test]
fn a_dialog_without_a_name_is_refused() {
    let refusals = support::refusals(
        "state showing is client Truth starting no\n\
         view\n\
         \x20   Dialog showing\n\
         \x20       Text \"Hello\"\n",
    );
    assert!(
        refusals
            .iter()
            .any(|message| message.contains("`Dialog` needs `label is")),
        "an unnamed modal must be refused: {refusals:?}"
    );
}

/// Whether the modal is showing is a `Truth`, and dismissing one must not
/// silently become a network write (§14B.5), so the signal is `client`.
#[test]
fn a_dialog_binds_a_client_truth_and_nothing_else() {
    let wrong_type = support::refusals(
        "state heading is client Text starting \"\"\n\
         view\n\
         \x20   Dialog heading, label is \"Confirm\"\n",
    );
    assert!(
        wrong_type
            .iter()
            .any(|message| message.contains("`Dialog` binds to")),
        "a modal bound to something other than a `Truth` must be refused: {wrong_type:?}"
    );
}
