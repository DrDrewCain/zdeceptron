//! A `Scene` gets its shape from the coordinate space it declares.
//!
//! HTML gives `<canvas>` an intrinsic 300x150 whatever box it is in, and
//! `width: 100%; height: 100%` only answers that when the parent has a
//! width and a height to be percentages *of*. In a flex column that sizes
//! itself from its contents — which is what a page is mostly made of —
//! the two rules are circular, the browser falls back to the intrinsic
//! box, and the drawing is rasterised at a size nothing in the program
//! asked for. It is not an exception and nothing throws: the picture is
//! correct, small, and the wrong shape.
//!
//! The fix is a fact the program already gave: `viewBox` **is** the
//! drawing's aspect ratio, so where it folds to a literal the compiler
//! writes the ratio into the generated class and the height follows from
//! the width. Nothing is circular, and no stylesheet has to repeat a
//! number the program already said.
//!
//! The assertions are on the emitted stylesheet because that is where the
//! answer lives — this is a rule the compiler writes, not a shape any
//! runtime computes. There is no case for a `Scene` without a `viewBox`:
//! it is a required argument, so the compiler always has a box to read
//! and the only question is whether it folds to a literal.

mod support;

use support::try_compile;

fn compile(source: &str) -> zdc_codegen::Bundle {
    try_compile(source, "test.zd").unwrap_or_else(|errors| panic!("test.zd: {}", errors[0].message))
}

/// The one generated class a program's single `Scene` carries.
fn scene_rule(bundle: &zdc_codegen::Bundle) -> String {
    let class = bundle
        .client_js
        .split("<canvas")
        .nth(1)
        .expect("this program draws a Scene, so its template holds a canvas")
        .split("class=\"")
        .nth(1)
        .expect("a Scene always carries a generated class: it always has sizing")
        .split('"')
        .next()
        .expect("the class attribute closes")
        .to_string();
    bundle
        .styles_css
        .lines()
        .find(|line| line.starts_with(&format!(".{class} {{")))
        .unwrap_or_else(|| {
            panic!(
                "`{class}` is on the canvas but not in the stylesheet:\n{}",
                bundle.styles_css
            )
        })
        .to_string()
}

const ONE_CIRCLE: &str = "        Circle x is 10, y is 10, radius is 4\n";

#[test]
fn a_literal_view_box_becomes_the_canvas_aspect_ratio() {
    let bundle = compile(&format!(
        "view\n    Scene viewBox is \"0 0 640 200\"\n{ONE_CIRCLE}"
    ));
    let rule = scene_rule(&bundle);
    assert!(
        rule.contains("aspect-ratio: 640 / 200"),
        "a 640x200 drawing should declare its own ratio, not inherit a box: {rule}"
    );
    assert!(
        rule.contains("height: auto"),
        "the height follows from the ratio, so `height: 100%` would fight it: {rule}"
    );
    assert!(
        rule.contains("max-height: 100%"),
        "a parent that *does* have a height still bounds it: {rule}"
    );
}

#[test]
fn the_ratio_is_the_view_boxs_size_and_not_its_corner() {
    // `minX minY width height`: a box whose origin is not 0,0 is the same
    // shape as one that is, and reading the first two numbers would give
    // a ratio out of a translation.
    let bundle = compile(&format!(
        "view\n    Scene viewBox is \"-40 -10 300 100\"\n{ONE_CIRCLE}"
    ));
    let rule = scene_rule(&bundle);
    assert!(
        rule.contains("aspect-ratio: 300 / 100"),
        "the ratio comes from the third and fourth numbers: {rule}"
    );
}

#[test]
fn two_scenes_of_different_shapes_do_not_share_a_class() {
    let bundle = compile(
        "view\n\
         \x20   Column\n\
         \x20       Scene viewBox is \"0 0 640 200\"\n\
         \x20           Circle x is 1, y is 1, radius is 1\n\
         \x20       Scene viewBox is \"0 0 100 100\"\n\
         \x20           Circle x is 1, y is 1, radius is 1\n",
    );
    assert!(
        bundle.styles_css.contains("aspect-ratio: 640 / 200")
            && bundle.styles_css.contains("aspect-ratio: 100 / 100"),
        "each drawing carries its own shape; interning them together would give one of them \
         the other's:\n{}",
        bundle.styles_css
    );
}

#[test]
fn a_degenerate_view_box_falls_back_rather_than_dividing_by_zero() {
    // `aspect-ratio: 640 / 0` is not a ratio, and a stylesheet is not the
    // place to find that out.
    let bundle = compile(&format!(
        "view\n    Scene viewBox is \"0 0 640 0\"\n{ONE_CIRCLE}"
    ));
    let rule = scene_rule(&bundle);
    assert!(
        !rule.contains("aspect-ratio"),
        "a zero-height box has no shape to declare: {rule}"
    );
    assert!(
        rule.contains("height: 100%"),
        "with no ratio to follow, the height is the one it was before: {rule}"
    );
}

#[test]
fn a_view_box_the_program_computes_declares_no_ratio() {
    // A stylesheet is written once and a signal is not: a ratio taken
    // from the first value of a cell that later changes would be a
    // number the page keeps asserting after it stopped being true.
    let bundle = compile(
        "state span is client Text starting \"0 0 640 200\"\n\
         view\n\
         \x20   Scene viewBox is span\n\
         \x20       Circle x is 10, y is 10, radius is 4\n",
    );
    let rule = scene_rule(&bundle);
    assert!(
        !rule.contains("aspect-ratio"),
        "the box is a signal, so its shape is not a constant: {rule}"
    );
}
