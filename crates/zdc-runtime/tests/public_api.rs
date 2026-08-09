use zdc_runtime::{eval_with_signals, BASE_CSS, DOM_JS, ELEMENTS_JS, LIST_JS, SIGNAL_JS};

#[test]
fn signal_updaters_execute_through_the_embedded_engine() {
    let result = eval_with_signals(
        r#"
        const [count, setCount] = signal(2);
        setCount(value => value * 3);
        count()
        "#,
    )
    .expect("script evaluates");

    assert_eq!(result, "6");
}

#[test]
fn each_evaluation_uses_a_fresh_javascript_context() {
    let first = eval_with_signals("globalThis.__zdc_probe = 41; __zdc_probe")
        .expect("first script evaluates");
    let second = eval_with_signals("typeof __zdc_probe").expect("second script evaluates");

    assert_eq!(first, "41");
    assert_eq!(second, "\"undefined\"");
}

#[test]
fn javascript_failures_retain_the_engine_message() {
    let error = eval_with_signals("throw new Error('runtime exploded')").unwrap_err();

    assert!(error.message.contains("runtime exploded"), "got: {error}");
    assert_eq!(error.to_string(), error.message);
}

#[test]
fn embedded_modules_keep_their_expected_linkage_boundaries() {
    assert!(SIGNAL_JS.contains("export function signal"));
    assert!(DOM_JS.contains("from './signal.js'"));
    assert!(ELEMENTS_JS.contains("from './dom.js'"));
    // One element, so the module is checked to export its vocabulary
    // rather than merely to import `dom.js`. There is no directory object
    // to assert against: `element_parity.rs` calls every name in the
    // vocabulary directly, which is a stronger check than a list.
    assert!(ELEMENTS_JS.contains("export function Column"));
}

/// Generated code never links `elements.js` (§16.3.1), so everything a
/// template emission calls has to be exported from a module a bundle does
/// link. That is `dom.js` for everything except keyed lists, which moved
/// to `list.js` so a program without one stops shipping the reconciler.
#[test]
fn the_template_surface_is_exported_from_dom_js() {
    for name in [
        "template",
        "bindText",
        "bindAttr",
        "bindStyle",
        "on",
        "anchors",
        "dynamicInto",
        "whenInto",
        // Node-position `if` (spec §14D.1's `Disclosure`), which the view
        // grammar gained with components.
        "ifInto",
    ] {
        assert!(
            DOM_JS.contains(&format!("export function {name}(")),
            "dom.js must export `{name}`"
        );
    }
    for name in ["each", "eachInto", "byPosition"] {
        assert!(
            LIST_JS.contains(&format!("export function {name}(")),
            "list.js must export `{name}`"
        );
        assert!(
            !DOM_JS.contains(&format!("export function {name}(")),
            "`{name}` moved to list.js; dom.js exporting it again would put \
             the reconciler back in every bundle"
        );
    }
    assert!(LIST_JS.contains("from './dom.js'"));
}

/// R6 moved the base styling out of JavaScript, so the declarations must
/// exist as CSS and must no longer be applied at runtime.
#[test]
fn base_styling_is_css_rather_than_inline_style() {
    for class in [".zd-col", ".zd-row", ".zd-err"] {
        assert!(BASE_CSS.contains(class), "base.css must define `{class}`");
    }
    assert!(
        !ELEMENTS_JS.contains("flex-direction"),
        "elements.js must not apply base styling at runtime any more"
    );
}
