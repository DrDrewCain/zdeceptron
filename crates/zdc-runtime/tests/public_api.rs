use zdc_runtime::{eval_with_signals, DOM_JS, ELEMENTS_JS, SIGNAL_JS};

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
    assert!(ELEMENTS_JS.contains("export const BUILTINS"));
}
