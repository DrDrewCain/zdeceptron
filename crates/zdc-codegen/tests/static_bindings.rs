//! A `static` value in every position that binds one.
//!
//! §14C.3b inlines a `static` signal as the literal the build host printed.
//! It is therefore **not** a getter, and every binding site that assumes a
//! non-literal operand must be one has the same defect: the emitted module
//! calls a string, and the element throws when it renders.
//!
//! Nothing here asserts about emitted text. A `static` in `class` emitted
//! `() => 'zd-col ' + ("accent")()`, which is a `TypeError` at render time
//! and not a bad-looking string, so the assertion is about the mounted DOM
//! — which is also why the tests survive a change to how the fix spells
//! its output.

mod support;

use std::collections::BTreeMap;

use support::{build_module_of, context, run, try_compile_with_statics};

/// Run the build root the way `zdc build` runs it, then compile with what
/// it printed — the same two steps `zdc build` takes for a `static`.
fn compile(source: &str) -> zdc_codegen::Bundle {
    let module = build_module_of(source, "test.zd")
        .expect("every program here declares `static` state, so it has a build root");
    let statics: BTreeMap<String, String> =
        zdc_codegen::evaluate(&module, std::path::Path::new("."))
            .unwrap_or_else(|error| panic!("the build root did not run: {}", error.report()))
            .values;
    try_compile_with_statics(source, "test.zd", statics)
        .unwrap_or_else(|errors| panic!("test.zd: {}", errors[0].message))
}

/// Mount the emitted module against the DOM shim and read the tree back.
///
/// A defect that throws inside `main` fails here with the engine's own
/// message, which is what makes this a demonstration rather than a string
/// comparison.
fn mounted(source: &str) -> String {
    let bundle = compile(source);
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    )
}

/// One CSS property of the first element with `tag`, as the DOM holds it.
fn style_of(source: &str, tag: &str, property: &str) -> String {
    let bundle = compile(source);
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        &format!(
            "const $host = document.createElement('div');\n\
             main($host);\n\
             String(walk($host).filter((n) => n !== $host)\
             .find((n) => n.tagName === '{tag}')\
             .style.properties['{property}'])"
        ),
    )
}

/// The demonstration. `palette` is `static`, so it is inlined as the text
/// `"accent"`; `class` emitted a getter that calls whatever it is given,
/// so the module rendered `("accent")()` and threw.
#[test]
fn a_static_signal_in_class_position_renders_its_text() {
    let dom = mounted(
        "state palette is static Text starting \"accent\"\n\
         view\n\
         \x20   Column class is palette\n\
         \x20       Text \"hello\"\n",
    );
    assert!(
        dom.contains("accent"),
        "the class the program asked for must reach the element:\n{dom}"
    );
    assert!(
        dom.contains("hello"),
        "the element must have rendered at all:\n{dom}"
    );
}

/// The other half of the same emission: a length style appends `px`, and
/// it appended it to a call.
///
/// Read off `node.style` rather than off the serialised tree: an inline
/// style is not an attribute, so `serialize` does not show it and an
/// assertion against the serialisation would pass on a style never set.
#[test]
fn a_static_signal_in_a_pixel_style_renders_its_length() {
    assert_eq!(
        style_of(
            "state gap is static Whole starting 8\n\
             view\n\
             \x20   Column padding is gap\n\
             \x20       Text \"hello\"\n",
            "div",
            "padding",
        ),
        "8px"
    );
}

/// A style that is not a length takes the value as it stands.
#[test]
fn a_static_signal_in_a_plain_style_renders_its_value() {
    assert_eq!(
        style_of(
            "state emphasis is static Text starting \"600\"\n\
             view\n\
             \x20   Column\n\
             \x20       Text \"hello\", weight is emphasis\n",
            "span",
            "font-weight",
        ),
        "600"
    );
}

/// Text position. The one site that was already right, tested anyway:
/// `Operand::Static` has its own arm there and binds once at clone time.
#[test]
fn a_static_signal_in_text_position_renders_its_text() {
    let dom = mounted(
        "state greeting is static Text starting \"hello\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text greeting\n",
    );
    assert!(dom.contains("hello"), "{dom}");
}

/// An ordinary attribute.
#[test]
fn a_static_signal_in_an_attribute_renders_its_value() {
    let dom = mounted(
        "state anchor is static Text starting \"top\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text \"hello\", id is anchor\n",
    );
    assert!(dom.contains("top"), "{dom}");
}

/// A URL attribute, which is filtered rather than merely set.
#[test]
fn a_static_signal_in_a_url_attribute_renders_its_url() {
    let dom = mounted(
        "state logo is static Text starting \"/logo.png\"\n\
         view\n\
         \x20   Column\n\
         \x20       Image source is logo, alt is \"a logo\"\n",
    );
    assert!(dom.contains("/logo.png"), "{dom}");
}

/// A link's destination is the same filter reached through the leading
/// slot rather than through a named argument.
#[test]
fn a_static_signal_in_a_link_destination_renders_its_url() {
    let dom = mounted(
        "state elsewhere is static Text starting \"https://example.com/x\"\n\
         view\n\
         \x20   Column\n\
         \x20       Link elsewhere\n\
         \x20           Text \"go\"\n",
    );
    assert!(dom.contains("https://example.com/x"), "{dom}");
}

/// An `each` over a `static` list: the list is a literal array, not a
/// getter, and `eachInto` reads it through `read`.
#[test]
fn a_static_list_is_iterated() {
    let dom = mounted(
        "state names is static List of Text starting [\"one\", \"two\"]\n\
         view\n\
         \x20   Column\n\
         \x20       each name in names\n\
         \x20           Text name\n",
    );
    assert!(dom.contains("one") && dom.contains("two"), "{dom}");
}

/// A `when` whose scrutinee is `static`.
///
/// The build host's answer is supplied by hand rather than computed. A
/// `static` holding a variant cannot be computed at all today — the build
/// root prints `variant('Busy')` and defines no `variant`, so `evaluate`
/// stops with E10 — and that refusal would hide the question this test
/// asks, which is what `whenInto` is handed once a value exists.
#[test]
fn a_static_scrutinee_selects_its_arm() {
    let source = "choice Status\n\
                  \x20   Idle\n\
                  \x20   Busy\n\
                  state status is static Status starting Busy\n\
                  view\n\
                  \x20   Column\n\
                  \x20       when status\n\
                  \x20           Idle show Text \"idle\"\n\
                  \x20           Busy show Text \"busy\"\n";
    let statics = BTreeMap::from([(
        "status".to_string(),
        r#"{"tag":"Busy","fields":[]}"#.to_string(),
    )]);
    let bundle = try_compile_with_statics(source, "test.zd", statics)
        .unwrap_or_else(|errors| panic!("test.zd: {}", errors[0].message));
    let mut context = context(false);
    let dom = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    );
    assert!(dom.contains("busy") && !dom.contains("idle"), "{dom}");
}

/// An `if` whose condition is `static`.
#[test]
fn a_static_condition_chooses_its_branch() {
    let dom = mounted(
        "state ready is static Truth starting yes\n\
         view\n\
         \x20   Column\n\
         \x20       if ready\n\
         \x20           Text \"shown\"\n",
    );
    assert!(dom.contains("shown"), "{dom}");
}

/// A `static` read from inside an event handler, which runs in a closure
/// the emitter builds rather than in the region's own scope.
#[test]
fn a_static_signal_read_by_a_handler_reaches_the_dom() {
    let bundle = compile(
        "state accent is static Text starting \"accent\"\n\
         state chosen is client Text starting \"\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text chosen\n\
         \x20       Button \"choose\"\n\
         \x20           on click\n\
         \x20               set chosen to accent\n",
    );
    let mut context = context(false);
    let dom = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\n\
         main($host);\n\
         walk($host).find((n) => n.tagName === 'button').fire('click');\n\
         serialize($host)",
    );
    assert!(dom.contains("accent"), "{dom}");
}

/// A `static` passed to a component, which binds it as that instance's
/// argument rather than reading it in the view's own region.
#[test]
fn a_static_signal_passed_to_a_component_reaches_the_dom() {
    let dom = mounted(
        "component Card with caption\n\
         \x20   Column\n\
         \x20       Text caption\n\
         state accent is static Text starting \"accent\"\n\
         view\n\
         \x20   Column\n\
         \x20       Card accent\n",
    );
    assert!(dom.contains("accent"), "{dom}");
}

/// A component's argument used in `class` position, which is the
/// defective site reached one region deeper.
#[test]
fn a_static_signal_passed_to_a_component_used_as_a_class() {
    let dom = mounted(
        "component Card with palette\n\
         \x20   Column class is palette\n\
         \x20       Text \"inside\"\n\
         state accent is static Text starting \"accent\"\n\
         view\n\
         \x20   Column\n\
         \x20       Card accent\n",
    );
    assert!(dom.contains("accent") && dom.contains("inside"), "{dom}");
}
