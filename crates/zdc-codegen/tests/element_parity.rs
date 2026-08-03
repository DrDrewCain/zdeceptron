//! The anti-drift test, per spec §16.3.6.
//!
//! The compiler owns the DOM shape of every built-in, which duplicates
//! `elements.js` — §16.10 names that as a known cost, and this is the whole
//! mechanism that keeps it from becoming a bug. For each built-in, with
//! constant arguments, the tree `elements.js` builds must `isEqualNode` the
//! tree the compiler's markup parses into.
//!
//! `elements.js` was verified in a browser and is no longer what ships.
//! That verification is inherited only through this test.

mod support;

use support::{compile_source, context};

use boa_engine::{Context, Source};

/// One case: the ZDeceptron view, and the `elements.js` call that must
/// produce the identical tree.
struct Case {
    element: &'static str,
    view: &'static str,
    reference: &'static str,
}

const CASES: &[Case] = &[
    Case {
        element: "Column",
        view: "view\n    Column\n        Text \"a\"\n",
        reference: "Column({}, [Text(() => 'a')])",
    },
    Case {
        element: "Row",
        view: "view\n    Row\n        Text \"a\"\n",
        reference: "Row({}, [Text(() => 'a')])",
    },
    Case {
        element: "Text",
        view: "view\n    Text \"hello\"\n",
        reference: "Text(() => 'hello')",
    },
    Case {
        element: "Heading",
        view: "view\n    Heading \"Title\"\n",
        reference: "Heading(() => 'Title')",
    },
    Case {
        element: "Button",
        view: "view\n    Button \"press\"\n",
        reference: "Button(() => 'press')",
    },
    Case {
        element: "Input",
        view: "state name is client Text starting \"world\"\nview\n    Input name, hint is \"your name\"\n",
        reference: "Input(signal('world'), { hint: 'your name' })",
    },
    Case {
        element: "Checkbox",
        view: "state done is client Truth starting no\nview\n    Checkbox done\n",
        reference: "Checkbox(signal(false))",
    },
    Case {
        element: "Checkbox with a label",
        view: "state done is client Truth starting no\nview\n    Checkbox done, label is \"ready\"\n",
        reference: "Checkbox(signal(false), { label: 'ready' })",
    },
    Case {
        element: "Spinner",
        view: "view\n    Spinner\n",
        reference: "Spinner()",
    },
    Case {
        element: "ErrorBar",
        view: "view\n    ErrorBar message is \"boom\"\n",
        reference: "ErrorBar({ message: 'boom' })",
    },
    // Routing's element. Its `href` is not written by the program: the
    // compiler renders it from the route value, which is what makes a
    // mistyped URL a name that does not resolve.
    Case {
        element: "Link",
        view: "route Site\n    Home is \"/\"\nview\n    Link Home\n        Text \"home\"\n",
        reference: "Link({ href: '/' }, [Text(() => 'home')])",
    },
];

/// The single `template('...')` literal out of an emitted module.
fn template_markup(client_js: &str) -> String {
    let start = client_js
        .find("template('")
        .unwrap_or_else(|| panic!("no template in:\n{client_js}"))
        + "template('".len();
    let rest = &client_js[start..];
    let end = rest
        .find("')")
        .unwrap_or_else(|| panic!("unterminated template in:\n{client_js}"));
    rest[..end].to_string()
}

fn assert_parity(context: &mut Context, case: &Case, markup: &str) {
    let script = format!(
        r#"
        (() => {{
          const built = {};
          const cloned = template({})();
          // `elements.js` returns one node; the compiler returns a fragment.
          const compiled = cloned.childNodes.length === 1 ? cloned.firstChild : cloned;
          if (built.isEqualNode(compiled)) return 'equal';
          return 'elements.js: ' + serialize(built) + '\ncompiler  : ' + serialize(compiled);
        }})()
        "#,
        case.reference,
        // The markup is already a JavaScript string literal's contents.
        format_args!("'{markup}'")
    );

    let verdict = context
        .eval(Source::from_bytes(script.as_bytes()))
        .unwrap_or_else(|e| panic!("{}: the parity script failed: {e}", case.element))
        .to_string(context)
        .expect("a string")
        .to_std_string_escaped();

    assert_eq!(
        verdict, "equal",
        "`{}` has drifted between the compiler's shape table and elements.js:\n{verdict}",
        case.element
    );
}

#[test]
fn every_built_in_renders_the_same_tree_through_both_strategies() {
    // One context holds both strategies: `elements.js` for the reference
    // tree, and `dom.js`'s `template` for the compiled one.
    let mut context = context(true);
    for case in CASES {
        let bundle = compile_source(case.view);
        let markup = template_markup(&bundle.client_js);
        assert_parity(&mut context, case, &markup);
    }
}

/// A test that stopped running its cases would report no drift at all.
#[test]
fn the_parity_suite_covers_every_built_in() {
    for built_in in zdc_codegen::BUILT_INS {
        assert!(
            CASES.iter().any(|case| case.element.starts_with(built_in)),
            "`{built_in}` has no parity case"
        );
    }
    assert!(CASES.len() >= 10, "expected at least ten assertions");
}
