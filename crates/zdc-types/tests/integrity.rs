//! The integrity pass (§18.1), through the compiler's public entry point.
//!
//! These live outside the crate so they run against the **real** placement
//! pass: `zdc-graph` implements §17.1.4's interface and depends on this
//! crate, so only a test binary can hold both. A stand-in here would be a
//! second copy of §14G.1.4's table, which is the drift the interface
//! exists to prevent.

fn errors(source: &str) -> Vec<String> {
    let program = zdc_parser::parse(source).expect("parses");
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect("resolves");
    let split = zdc_graph::split(&hir);
    match zdc_types::check(&hir, &split) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|error| error.message).collect(),
    }
}

#[test]
fn a_program_that_never_writes_the_word_is_checked_no_differently() {
    let found = errors(
        "state count is client Whole starting 0\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           add 1 to count\n",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn trusted_client_is_rejected_at_the_declaration() {
    let found = errors("trusted state n is client Whole starting 0\nview\n    Text n\n");
    assert!(
        found.iter().any(|m| m.contains("E-INT-01")),
        "expected E-INT-01: {found:?}"
    );
}

/// The acceptance case: an event payload written into a `trusted`
/// place, and the diagnostic names the payload rather than the write.
#[test]
fn an_event_payload_may_not_be_written_to_a_trusted_place() {
    let found = errors(
        "trusted state note is durable Text starting \"\"\n\
         view\n\
         \x20   Input\n\
         \x20       on keydown with press\n\
         \x20           set note to press.key\n",
    );
    assert!(
        found.iter().any(|m| m.contains("E-INT-03")),
        "expected E-INT-03: {found:?}"
    );
    assert!(
        found
            .iter()
            .any(|m| m.contains("press") && m.contains("keydown")),
        "the diagnostic must name the payload: {found:?}"
    );
}

/// A payload reaching the *index* of a trusted place is the IDOR shape,
/// and it is E-INT-02 rather than E-INT-03.
#[test]
fn an_event_payload_may_not_choose_which_entry_is_written() {
    let found = errors(
        "trusted state moderators is durable Map of Text to Truth starting empty\n\
         view\n\
         \x20   Button \"promote\"\n\
         \x20       on click with press\n\
         \x20           set moderators at press.x to yes\n",
    );
    assert!(
        found.iter().any(|m| m.contains("E-INT-02")),
        "expected E-INT-02: {found:?}"
    );
}

/// The lattice discriminates. In a server-rooted body — the one place
/// §18.1 says obligations live — `environment` is trusted and a lifted
/// client signal is not, and the same write is accepted or refused on
/// that difference alone.
#[test]
fn a_server_rooted_write_is_judged_on_where_the_value_came_from() {
    let refused = errors(
        "trusted state moderators is durable Map of Text to Truth starting empty\n\
         state candidate is client Text starting \"\"\n\
         state promoted is server Truth from promote with candidate\n\
         function promote with who\n\
         \x20   set moderators at who to yes\n\
         \x20   give yes\n\
         view\n\
         \x20   Input candidate\n",
    );
    assert!(
        refused.iter().any(|m| m.contains("E-INT-02")),
        "a lifted client value must not choose the entry: {refused:?}"
    );

    let accepted = errors(
        "trusted state moderators is durable Map of Text to Truth starting empty\n\
         state root is server Text from environment \"ROOT\"\n\
         state promoted is server Truth from promote with root\n\
         function promote with who\n\
         \x20   set moderators at who to yes\n\
         \x20   give yes\n\
         view\n\
         \x20   Text \"x\"\n",
    );
    assert!(
        accepted.is_empty(),
        "an operator-set value is trusted: {accepted:?}"
    );
}

/// §18.1 semantics 11 — the implicit flow. The value written is a
/// literal; the decision to write it is not.
#[test]
fn a_write_decided_by_an_untrusted_value_is_rejected() {
    let found = errors(
        "trusted state moderators is durable Map of Text to Truth starting empty\n\
         state wanted is client Truth starting no\n\
         state promoted is server Truth from promote with wanted\n\
         function promote with asked\n\
         \x20   if asked\n\
         \x20       set moderators at \"root\" to yes\n\
         \x20   give yes\n\
         view\n\
         \x20   Checkbox wanted\n",
    );
    assert!(
        found.iter().any(|m| m.contains("E-INT-04")),
        "expected E-INT-04: {found:?}"
    );
}
