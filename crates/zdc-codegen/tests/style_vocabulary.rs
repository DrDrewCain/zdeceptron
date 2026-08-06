//! The styling vocabulary: one value grammar per style argument.
//!
//! Every assertion here goes through the *mounted* DOM to find the class an
//! element actually carries, and then through `styles.css` to read what
//! that class declares. A test that read the emitted text instead would
//! pass for a class name printed into markup that no rule matches, which is
//! not styling.
//!
//! The organising rule of the whole vocabulary, and the reason each
//! argument is a separate decision rather than a row in one table: a style
//! value is **printed** into a folded stylesheet, so a value that can close
//! its own declaration writes CSS for the whole page. There is no CSS
//! escape that keeps a value meaning what it said, so each argument names
//! what it admits: a colour, a length, one word from a closed set. Every
//! other value is refused. `injection.rs` holds the matching refusals.

mod support;

use support::{compile_source, context, refusals, run};

/// The class list of the first `tag` in the mounted tree.
fn classes(bundle: &zdc_codegen::Bundle, tag: &str) -> Vec<String> {
    let mut context = context(false);
    let text = run(
        &mut context,
        &bundle.client_js,
        &format!(
            "const $host = document.createElement('div');\n\
             main($host);\n\
             String(walk($host).filter((n) => n !== $host)\
             .find((n) => n.tagName === '{tag}')\
             .attributes['class'] ?? '')"
        ),
    );
    text.split_whitespace().map(str::to_string).collect()
}

/// The generated class the first `tag` carries, or a panic naming what it
/// carried instead.
fn generated_class(bundle: &zdc_codegen::Bundle, tag: &str) -> String {
    let classes = classes(bundle, tag);
    classes
        .iter()
        .find(|class| class.starts_with("zd-s"))
        .unwrap_or_else(|| panic!("`{tag}` carries no generated class: {classes:?}"))
        .clone()
}

/// Every rule in `styles.css` whose selector begins with the element's
/// generated class, joined, so an assertion can name a declaration
/// without depending on how the sheet lays its rules out.
fn rules(bundle: &zdc_codegen::Bundle, tag: &str) -> String {
    let class = generated_class(bundle, tag);
    let needle = format!(".{class}");
    bundle
        .styles_css
        .lines()
        .filter(|line| line.contains(&needle))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Compile `source`, and return what the first `tag` in it is styled with.
fn styled(source: &str, tag: &str) -> String {
    let bundle = compile_source(source);
    rules(&bundle, tag)
}

/// A view holding one `Text` with the given arguments.
fn text_with(arguments: &str) -> String {
    format!("view\n    Column\n        Text \"x\", {arguments}\n")
}

fn assert_refused(source: &str, needle: &str) {
    let messages = refusals(source);
    assert!(
        messages.iter().any(|message| message.contains(needle)),
        "expected a diagnostic mentioning `{needle}`, got:\n{}",
        messages.join("\n")
    );
}

// ---------------------------------------------------------------------
// #64 Colour.
//
// A colour is a hex triple or one of the plain colour words. It is
// deliberately not "any CSS colour": `rgb(…)` and `color-mix(…)` are
// function calls, and a function call in a printed declaration is a
// parenthesis a value can walk out of.
// ---------------------------------------------------------------------

#[test]
fn a_named_colour_folds_into_the_generated_class() {
    let rules = styled(&text_with("color is \"red\""), "span");
    assert!(rules.contains("color: red;"), "{rules}");
}

#[test]
fn a_hex_colour_folds_into_the_generated_class() {
    let rules = styled(&text_with("color is \"#b3151c\""), "span");
    assert!(rules.contains("color: #b3151c;"), "{rules}");
}

#[test]
fn a_short_hex_colour_folds_into_the_generated_class() {
    let rules = styled(&text_with("color is \"#abc\""), "span");
    assert!(rules.contains("color: #abc;"), "{rules}");
}

#[test]
fn a_colour_that_is_not_a_colour_is_refused() {
    assert_refused(&text_with("color is \"reddish\""), "is a colour");
}

#[test]
fn a_colour_may_not_be_a_function_call() {
    assert_refused(&text_with("color is \"rgb(1,2,3)\""), "is a colour");
}

#[test]
fn a_hex_colour_of_the_wrong_length_is_refused() {
    assert_refused(&text_with("color is \"#ab\""), "is a colour");
}

// ---------------------------------------------------------------------
// #65 Background.
//
// Two arguments, because they are two different values: a colour and a
// URL. `background is "/a.png"` reading as an image would mean guessing
// which of the two a string was, and a guess is what a closed grammar is
// for avoiding.
// ---------------------------------------------------------------------

#[test]
fn a_background_colour_folds_into_the_generated_class() {
    let rules = styled(
        "view\n    Column background is \"#f5f5f5\"\n        Text \"x\"\n",
        "div",
    );
    assert!(rules.contains("background-color: #f5f5f5;"), "{rules}");
}

#[test]
fn a_backdrop_becomes_a_background_image_through_the_url_sink() {
    let rules = styled(
        "view\n    Column backdrop is \"/hero.png\"\n        Text \"x\"\n",
        "div",
    );
    assert!(rules.contains("background-image: url(/hero.png);"), "{rules}");
}

/// The same list `Image` and `Link` are held to. A background image is a
/// request the browser issues, so `javascript:` is refused here for the
/// reason it is refused there.
#[test]
fn a_backdrop_may_not_name_a_script_url() {
    assert_refused(
        "view\n    Column backdrop is \"javascript:alert(1)\"\n        Text \"x\"\n",
        "`backdrop` is",
    );
}

/// `url(…)` is printed, so a value carrying a parenthesis leaves the
/// function it was written into and names a second one. `url_is_safe`
/// does not catch this: the value names no scheme at all, so it is a
/// relative URL as far as that check is concerned.
#[test]
fn a_backdrop_may_not_close_the_url_it_is_printed_into() {
    assert_refused(
        "view\n    Column backdrop is \"/a.png), url(https://evil.example/x\"\n        Text \"x\"\n",
        "`backdrop` is",
    );
}

/// Whitespace is not a URL character, and a `url()` token ends at the
/// first space, so a value carrying one would close the function early
/// and leave the rest of itself as CSS.
#[test]
fn a_backdrop_may_not_carry_whitespace() {
    assert_refused(
        "view\n    Column backdrop is \"/a b.png\"\n        Text \"x\"\n",
        "`backdrop` is",
    );
}

/// A backdrop is literal-only. The reactive path would have to build
/// `url("…")` around a runtime value, and there is no runtime check that
/// keeps a value inside the parentheses it is printed between: `safeUrl`
/// rules on schemes, not on delimiters.
#[test]
fn a_backdrop_that_is_not_written_down_is_refused() {
    assert_refused(
        "state spot is client Text starting \"/a.png\"\n\
         view\n\
         \x20   Column backdrop is spot\n\
         \x20       Text \"x\"\n",
        "written down",
    );
}

// ---------------------------------------------------------------------
// #66 Margin.
//
// The same length grammar `padding` has always had, widened to the one-to
// -four form CSS's own shorthand takes, because "space above and below but
// not beside" is the commonest margin there is.
// ---------------------------------------------------------------------

#[test]
fn a_margin_folds_into_the_generated_class_beside_padding() {
    let rules = styled(
        "view\n    Column margin is 16, padding is 8\n        Text \"x\"\n",
        "div",
    );
    assert!(rules.contains("margin: 16px;"), "{rules}");
    assert!(rules.contains("padding: 8px;"), "{rules}");
}

#[test]
fn a_margin_takes_the_one_to_four_shorthand() {
    let rules = styled(
        "view\n    Column margin is \"16 0\"\n        Text \"x\"\n",
        "div",
    );
    assert!(rules.contains("margin: 16px 0px;"), "{rules}");
}

#[test]
fn a_margin_of_more_than_four_lengths_is_refused() {
    assert_refused(
        "view\n    Column margin is \"1 2 3 4 5\"\n        Text \"x\"\n",
        "`margin` is",
    );
}

#[test]
fn a_margin_that_carries_its_own_unit_is_refused() {
    assert_refused(
        "view\n    Column margin is \"16px\"\n        Text \"x\"\n",
        "`margin` is",
    );
}

// ---------------------------------------------------------------------
// #67 Border.
//
// Width, style and colour, as three arguments rather than one shorthand
// string. A shorthand would be `border is "1px solid #ccc"`, which is a
// CSS declaration value written by the program: exactly the thing this
// vocabulary exists so that nobody has to write.
//
// `border is 1` declares `solid` alongside the width, because a border
// with no style is not drawn at all and a width alone would render
// nothing and read as a compiler bug. `borderStyle` follows it in the
// sheet and overrides it, which is a property of the printed order and is
// pinned below rather than assumed.
// ---------------------------------------------------------------------

#[test]
fn a_border_width_renders_a_border() {
    let rules = styled("view\n    Column border is 1\n        Text \"x\"\n", "div");
    assert!(rules.contains("border: 1px solid;"), "{rules}");
}

#[test]
fn a_border_takes_a_colour_of_its_own() {
    let rules = styled(
        "view\n    Column border is 1, borderColor is \"grey\"\n        Text \"x\"\n",
        "div",
    );
    assert!(rules.contains("border-color: grey;"), "{rules}");
}

#[test]
fn a_border_style_follows_the_width_it_overrides() {
    let rules = styled(
        "view\n    Column border is 2, borderStyle is \"dashed\"\n        Text \"x\"\n",
        "div",
    );
    assert!(rules.contains("border: 2px solid;"), "{rules}");
    assert!(rules.contains("border-style: dashed;"), "{rules}");
    assert!(
        rules.find("border: 2px solid;") < rules.find("border-style: dashed;"),
        "the declared style must follow the shorthand or it never wins:\n{rules}"
    );
}

#[test]
fn a_border_style_outside_the_closed_set_is_refused() {
    assert_refused(
        "view\n    Column borderStyle is \"groove\"\n        Text \"x\"\n",
        "`borderStyle` is",
    );
}

#[test]
fn a_border_width_that_carries_its_own_unit_is_refused() {
    assert_refused(
        "view\n    Column border is \"1px\"\n        Text \"x\"\n",
        "`border` is",
    );
}

#[test]
fn a_border_colour_that_is_not_a_colour_is_refused() {
    assert_refused(
        "view\n    Column borderColor is \"solid\"\n        Text \"x\"\n",
        "`borderColor` is",
    );
}

// ---------------------------------------------------------------------
// #68 Radius.
//
// The uniform case is a number and the per-corner case is up to four of
// them, which is CSS's own shorthand order: top-left, top-right,
// bottom-right, bottom-left. The elliptical form, `8px / 4px`, is not
// expressible, and deliberately: it needs a slash inside the value, and a
// value grammar that admits a delimiter is a value grammar with a
// delimiter in it.
// ---------------------------------------------------------------------

#[test]
fn a_uniform_radius_rounds_every_corner() {
    let rules = styled("view\n    Column radius is 8\n        Text \"x\"\n", "div");
    assert!(rules.contains("border-radius: 8px;"), "{rules}");
}

#[test]
fn a_per_corner_radius_rounds_the_corners_it_names() {
    let rules = styled(
        "view\n    Column radius is \"8 8 0 0\"\n        Text \"x\"\n",
        "div",
    );
    assert!(rules.contains("border-radius: 8px 8px 0px 0px;"), "{rules}");
}

#[test]
fn a_radius_may_not_carry_a_slash() {
    assert_refused(
        "view\n    Column radius is \"8 / 4\"\n        Text \"x\"\n",
        "`radius` is",
    );
}

#[test]
fn a_radius_of_more_than_four_corners_is_refused() {
    assert_refused(
        "view\n    Column radius is \"1 2 3 4 5\"\n        Text \"x\"\n",
        "`radius` is",
    );
}

// ---------------------------------------------------------------------
// #69 Display.
//
// Four words, and `flex` is not among them. `Row` and `Column` *are* the
// flex containers: a second way to make one would be the two-phrasings
// problem §4.1 forbids, and `display is "flex"` on a `Text` would be a
// flex container with no way to say which way it runs.
//
// What `display` interacts with is exactly those two elements, and the
// interaction is decided by print order rather than by specificity: every
// generated rule carries one class, `.zd-row` carries one class, and the
// generated rules are printed after `base.css`. So a program's `display`
// wins on a `Row`, and the test below says so rather than leaving it to
// be discovered.
// ---------------------------------------------------------------------

#[test]
fn display_folds_into_the_generated_class() {
    let rules = styled(
        "view\n    Column\n        Text \"x\", display is \"block\"\n",
        "span",
    );
    assert!(rules.contains("display: block;"), "{rules}");
}

/// A `Row` is a flex container by its base class, and a program that says
/// otherwise is obeyed. This is the interaction #69 asked to have written
/// down: it holds because the generated rules follow `base.css` and both
/// selectors carry one class of specificity.
#[test]
fn a_display_on_a_row_beats_the_base_class_it_shares_specificity_with() {
    let bundle = compile_source("view\n    Row display is \"block\"\n        Text \"x\"\n");
    let class = generated_class(&bundle, "div");
    let sheet = &bundle.styles_css;
    let base = sheet.find(".zd-row").expect("the base class ships");
    let generated = sheet
        .find(&format!(".{class} {{"))
        .expect("the generated class ships");
    assert!(
        base < generated,
        "the generated rule must be printed after `base.css`:\n{sheet}"
    );
    assert!(
        sheet[generated..].starts_with(&format!(".{class} {{ display: block; }}")),
        "{sheet}"
    );
}

/// `Row` and `Column` are how a flex container is written. A second
/// spelling would be §4.1's two-phrasings problem, so the word is not in
/// the set at all.
#[test]
fn display_cannot_name_a_flex_container() {
    assert_refused(
        "view\n    Column display is \"flex\"\n        Text \"x\"\n",
        "`display` is",
    );
}

#[test]
fn a_display_outside_the_closed_set_is_refused() {
    assert_refused(
        "view\n    Column display is \"table-caption\"\n        Text \"x\"\n",
        "`display` is",
    );
}

// ---------------------------------------------------------------------
// #70 Flex.
//
// Three arguments and not the `flex` shorthand, because the shorthand's
// one-value form means three different things depending on whether the
// value has a unit: `flex: 1` is `1 1 0%` and `flex: 10px` is `1 1 10px`.
// A grammar whose meaning turns on the unit is a grammar that
// cannot be read.
// ---------------------------------------------------------------------

#[test]
fn a_child_declares_how_it_shares_the_space() {
    let rules = styled("view\n    Column grow is 1\n        Text \"x\"\n", "div");
    assert!(rules.contains("flex-grow: 1;"), "{rules}");
}

/// The layout #70 names: a fixed sidebar beside a column that takes what
/// is left.
#[test]
fn a_two_column_layout_with_a_fixed_sidebar_is_writable() {
    let bundle = compile_source(
        "view\n\
         \x20   Row\n\
         \x20       Column basis is 240, shrink is 0\n\
         \x20           Text \"nav\"\n\
         \x20       Column grow is 1\n\
         \x20           Text \"body\"\n",
    );
    let sheet = &bundle.styles_css;
    assert!(sheet.contains("flex-basis: 240px;"), "{sheet}");
    assert!(sheet.contains("flex-shrink: 0;"), "{sheet}");
    assert!(sheet.contains("flex-grow: 1;"), "{sheet}");
}

#[test]
fn a_grow_that_is_not_a_number_is_refused() {
    assert_refused(
        "view\n    Column grow is \"auto\"\n        Text \"x\"\n",
        "`grow` is",
    );
}

#[test]
fn a_basis_that_carries_its_own_unit_is_refused() {
    assert_refused(
        "view\n    Column basis is \"240px\"\n        Text \"x\"\n",
        "`basis` is",
    );
}

// ---------------------------------------------------------------------
// #71 Justify and align.
//
// The words are `start`, `end`, `center`, `between`, `around` and
// `evenly`, which is what a person says. CSS's own spellings are
// `flex-start` and `space-between`, and a program that had to write those
// would be writing CSS with extra steps.
//
// `justify` is along the direction the container runs, `align` is across
// it. That is CSS's model and the naming does not hide it, because
// hiding it would mean a `Row` and a `Column` disagreeing about which
// argument centres horizontally.
// ---------------------------------------------------------------------

#[test]
fn content_centres_on_both_axes() {
    let bundle = compile_source(
        "view\n    Row justify is \"center\", align is \"center\"\n        Text \"x\"\n",
    );
    let rules = rules(&bundle, "div");
    assert!(rules.contains("justify-content: center;"), "{rules}");
    assert!(rules.contains("align-items: center;"), "{rules}");
}

#[test]
fn the_distribution_words_read_as_english() {
    let rules = styled(
        "view\n    Row justify is \"between\"\n        Text \"x\"\n",
        "div",
    );
    assert!(rules.contains("justify-content: space-between;"), "{rules}");
}

/// The CSS spelling is not a second way to say it. §4.1 forbids two
/// phrasings for one construct, and the translation table is the whole
/// reason the argument exists.
#[test]
fn the_css_spelling_of_a_distribution_is_not_accepted() {
    assert_refused(
        "view\n    Row justify is \"space-between\"\n        Text \"x\"\n",
        "`justify` is",
    );
    assert_refused(
        "view\n    Row align is \"flex-start\"\n        Text \"x\"\n",
        "`align` is",
    );
}

// ---------------------------------------------------------------------
// #72 Gap.
//
// One declaration on the container, which is the thing flexbox's `gap`
// exists to replace: padding on each child breaks the moment a child is
// conditional, and `Row` and `Column` already declare a default gap in
// `base.css` that a program had no way to change.
// ---------------------------------------------------------------------

#[test]
fn a_container_declares_the_gap_between_its_children_once() {
    let rules = styled(
        "view\n    Row gap is 16\n        Text \"a\"\n        Text \"b\"\n",
        "div",
    );
    assert!(rules.contains("gap: 16px;"), "{rules}");
}

/// Two lengths are the row gap and the column gap, in that order, which
/// is CSS's own shorthand.
#[test]
fn a_gap_takes_a_row_and_a_column_measure() {
    let rules = styled(
        "view\n    Row gap is \"8 16\"\n        Text \"a\"\n",
        "div",
    );
    assert!(rules.contains("gap: 8px 16px;"), "{rules}");
}

/// `base.css` gives `Row` a gap and the generated rules follow it, so a
/// declared gap replaces the default rather than being ignored.
#[test]
fn a_declared_gap_beats_the_base_class_default() {
    let bundle = compile_source("view\n    Row gap is 0\n        Text \"a\"\n");
    let class = generated_class(&bundle, "div");
    let sheet = &bundle.styles_css;
    assert!(
        sheet.find(".zd-row").expect("base") < sheet.find(&format!(".{class} {{")).expect("gen"),
        "{sheet}"
    );
    assert!(sheet.contains(&format!(".{class} {{ gap: 0px; }}")), "{sheet}");
}

#[test]
fn a_gap_that_is_not_a_length_is_refused() {
    assert_refused(
        "view\n    Row gap is \"wide\"\n        Text \"a\"\n",
        "`gap` is",
    );
}

// ---------------------------------------------------------------------
// #87 Width and height, with a minimum and a maximum.
//
// Six arguments, all lengths in pixels. `Image` and `Canvas` keep their
// `width` and `height` *attributes*, and that is not two meanings for one
// name pretending to be one: an `img` with those attributes reserves its
// layout box before the file arrives, which is what stops a page
// reflowing as images load, and no stylesheet rule can do it because the
// rule does not know the aspect ratio.
// ---------------------------------------------------------------------

#[test]
fn any_element_takes_a_size() {
    let bundle = compile_source(
        "view\n    Column width is 320, height is 200\n        Text \"x\"\n",
    );
    let rules = rules(&bundle, "div");
    assert!(rules.contains("width: 320px;"), "{rules}");
    assert!(rules.contains("height: 200px;"), "{rules}");
}

/// The measure #87 asks for: a text column that stops at a readable
/// width however wide the window is.
#[test]
fn a_maximum_reading_width_is_expressible() {
    let rules = styled(
        "view\n    Column maxWidth is 720\n        Paragraph \"long\"\n",
        "div",
    );
    assert!(rules.contains("max-width: 720px;"), "{rules}");
}

#[test]
fn a_minimum_and_a_maximum_fold_beside_each_other() {
    let bundle = compile_source(
        "view\n    Column minWidth is 200, maxHeight is 400, minHeight is 40\n        Text \"x\"\n",
    );
    let rules = rules(&bundle, "div");
    assert!(rules.contains("min-width: 200px;"), "{rules}");
    assert!(rules.contains("min-height: 40px;"), "{rules}");
    assert!(rules.contains("max-height: 400px;"), "{rules}");
}

/// An image's intrinsic size is an attribute and stays one, so the
/// browser can reserve the box before the bytes arrive.
#[test]
fn an_image_sizes_itself_through_attributes_rather_than_a_class() {
    let bundle = compile_source(
        "view\n    Column\n        Image source is \"/a.png\", alt is \"a\", width is 64, height is 64\n",
    );
    let mut context = context(false);
    let attributes = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         const $img = walk($host).find((n) => n.tagName === 'img');\n\
         String($img.attributes['width']) + 'x' + String($img.attributes['height'])",
    );
    assert_eq!(attributes, "64x64");
}

#[test]
fn a_width_that_is_not_a_length_is_refused() {
    assert_refused(
        "view\n    Column width is \"100%\"\n        Text \"x\"\n",
        "`width` is",
    );
}

// ---------------------------------------------------------------------
// #88 Font family.
//
// Four words naming four stacks the compiler writes. A program cannot
// name a family directly, and that is the decision: a family name is
// arbitrary text that ends up in a printed declaration, it needs quoting
// the moment it has a space in it, and quoting a value inside a printed
// rule is the shape of every injection this compiler has had.
//
// A font *file* is a separate question and it already has an answer:
// `assets/` copies anything, and an `assets/*.css` carrying `@font-face`
// is linked after the generated sheet. See the assets test below.
// ---------------------------------------------------------------------

#[test]
fn a_typeface_is_selectable() {
    let rules = styled(
        "view\n    Column font is \"serif\"\n        Text \"x\"\n",
        "div",
    );
    assert!(rules.contains("font-family: ui-serif,"), "{rules}");
}

#[test]
fn every_font_word_names_a_stack_that_ends_in_a_generic_family() {
    let mut checked = 0;
    for (word, generic) in [
        ("system", "sans-serif"),
        ("sans", "sans-serif"),
        ("serif", "serif"),
        ("mono", "monospace"),
    ] {
        checked += 1;
        let rules = styled(
            &format!("view\n    Column font is \"{word}\"\n        Text \"x\"\n"),
            "div",
        );
        assert!(
            rules.contains(&format!("{generic};")),
            "`{word}` must fall back to `{generic}`:\n{rules}"
        );
    }
    assert_eq!(checked, 4, "every font word must be checked");
}

/// A family name is arbitrary text in a printed declaration, and the
/// moment it has a space in it, it needs quoting. There is no argument
/// that takes one.
#[test]
fn a_font_family_cannot_be_named_directly() {
    assert_refused(
        "view\n    Column font is \"Comic Sans MS\"\n        Text \"x\"\n",
        "`font` is",
    );
}

// ---------------------------------------------------------------------
// #89 Font size, against a declared scale.
//
// Six names, not free numbers. A number at every use site is how a
// document ends up with `13px`, `13.5px` and `14px` doing one job; the
// scale is declared once in `base.css` as custom properties, so it is one
// thing in one place and a program can retune it from an `assets/*.css`.
// ---------------------------------------------------------------------

#[test]
fn a_size_names_a_step_on_the_scale() {
    let bundle = compile_source("view\n    Column\n        Text \"x\", size is \"large\"\n");
    let rules = rules(&bundle, "span");
    assert!(rules.contains("font-size: var(--zd-text-large);"), "{rules}");
    assert!(
        bundle.styles_css.contains("--zd-text-large:"),
        "the scale must be declared, or the class names nothing:\n{}",
        bundle.styles_css
    );
}

#[test]
fn every_step_of_the_scale_is_declared() {
    let bundle = compile_source("view\n    Column\n        Text \"x\", size is \"tiny\"\n");
    let mut checked = 0;
    for step in ["tiny", "small", "normal", "large", "huge", "giant"] {
        checked += 1;
        assert!(
            bundle.styles_css.contains(&format!("--zd-text-{step}:")),
            "`{step}` is a size a program can name but the scale does not declare it:\n{}",
            bundle.styles_css
        );
    }
    assert_eq!(checked, 6, "every step must be checked");
}

#[test]
fn a_size_in_pixels_is_refused() {
    assert_refused(&text_with("size is \"14px\""), "`size` is");
}

#[test]
fn a_size_outside_the_scale_is_refused() {
    assert_refused(&text_with("size is \"medium\""), "`size` is");
}
