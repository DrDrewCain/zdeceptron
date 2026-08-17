//! `unique` names a row's identity, and `each` keys on it (#2, #367).
//!
//! # Why this is a correctness test and not a performance one
//!
//! `runtime/list.js` states the rule and refuses to default `keyOf`:
//!
//! > Keys are required, not optional. Without identity, reordering
//! > destroys and recreates nodes, which loses focus, scroll position, and
//! > the contents of any input inside a row. That is a correctness bug,
//! > not a performance one, which is why `keyOf` has no default.
//!
//! The emitter supplied that default at every call site anyway, because
//! `unique` was refused past the parser — so the runtime's stated
//! requirement was unsatisfiable by any source program. These tests pin
//! the two halves of the fix: a record that declares an identity is keyed
//! on it, and one that does not still reconciles by position.

mod support;

use support::compile_source;

const KEYED: &str = "\
record Row
    unique id is Text
    label is Text

state rows is client List of Row starting []

view
    Column
        each row in rows
            Text row.label
";

const POSITIONAL: &str = "\
record Row
    id is Text
    label is Text

state rows is client List of Row starting []

view
    Column
        each row in rows
            Text row.label
";

#[test]
fn a_record_with_an_identity_is_keyed_on_it() {
    let bundle = compile_source(KEYED);
    assert!(
        bundle.client_js.contains("(item) => item.id"),
        "the reconciler does not read the identity field:\n{}",
        bundle.client_js
    );
    // And the positional default is not declared at all, so a program
    // whose every list is keyed carries no dead helper.
    assert!(
        !bundle.client_js.contains("$byPosition"),
        "the positional default is still emitted beside a keyed list:\n{}",
        bundle.client_js
    );
}

#[test]
fn a_record_without_one_still_reconciles_by_position() {
    let bundle = compile_source(POSITIONAL);
    assert!(
        bundle.client_js.contains("$byPosition"),
        "a list with no identity must keep the positional reconciler:\n{}",
        bundle.client_js
    );
    assert!(
        !bundle.client_js.contains("(item) => item."),
        "nothing declared an identity, so nothing should be keyed:\n{}",
        bundle.client_js
    );
}

/// A list of something that is not a record has no field to key on, and
/// asking the type table for one must not confuse it for a record.
#[test]
fn a_list_of_text_reconciles_by_position() {
    let bundle = compile_source(
        "state names is client List of Text starting []\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       each name in names\n\
         \x20           Text name\n",
    );
    assert!(
        bundle.client_js.contains("$byPosition"),
        "a list of Text has no identity to key on:\n{}",
        bundle.client_js
    );
}
