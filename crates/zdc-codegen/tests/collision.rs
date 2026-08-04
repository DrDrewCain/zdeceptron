//! A program may name its state anything, and the bundle must still run.
//!
//! §16.3.2's guarantee is that a generated name cannot collide with a
//! user one, and it rested on every generated name beginning with `$`.
//! The names the emission *imports* — `bindText`, `template`, `signal` —
//! never did, and neither do `main` and its `container` parameter. This
//! runs the bundle rather than reading it, because two of the three
//! failures are silent in the text: `state container` produced a module
//! that parsed, loaded, and rendered the host element.

mod support;

use support::{compile_source, context, run};

/// Every unaliased runtime import a program could also spell, the entry
/// point, and its parameter. `on` is absent because it is a ZDeceptron
/// keyword, so no program can declare it.
const COLLIDING: &[&str] = &[
    "signal",
    "derived",
    "template",
    "variant",
    "mount",
    "bindText",
    "bindAttr",
    "bindStyle",
    "anchors",
    "safeUrl",
    "eachInto",
    "ifInto",
    "whenInto",
    "main",
    "container",
];

/// The bundle loads, and the value shown is the program's state rather
/// than whatever the emitted module happened to bind that name to.
#[test]
fn a_state_named_after_an_emitted_binding_still_runs() {
    for name in COLLIDING {
        let bundle = compile_source(&format!(
            "state {name} is client Whole starting 7\n\nview\n    Column\n        Text {name}\n"
        ));
        let mut engine = context(false);
        let rendered = run(
            &mut engine,
            &bundle.client_js,
            "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
        );
        assert!(
            rendered.contains(">7<"),
            "`state {name}` rendered `{rendered}` rather than its own value:\n{}",
            bundle.client_js
        );
    }
}

/// The setter is reserved with the getter, so a written signal named
/// after an import is two fresh identifiers rather than one.
#[test]
fn a_written_state_named_after_an_emitted_binding_still_runs() {
    let bundle = compile_source(
        r#"
state template is client Whole starting 0

view
    Column
        Text template
        Button "add"
            on click
                add 1 to template
"#,
    );
    let mut engine = context(false);
    let rendered = run(
        &mut engine,
        &bundle.client_js,
        r#"
const $host = document.createElement('div');
main($host);
walk($host).filter((n) => n.tagName === 'button')[0].fire('click');
serialize($host)
"#,
    );
    assert!(
        rendered.contains(">1<"),
        "the click did not reach the program's own signal: {rendered}\n{}",
        bundle.client_js
    );
}
