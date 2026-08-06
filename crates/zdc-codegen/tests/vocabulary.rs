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
    let bundle = compile_source(source);
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    )
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
