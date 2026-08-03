//! What the integrity pass (spec §18.1) accepts, refuses, and says.
//!
//! Driven through `zdc_types::check`, so what is asserted here is what a
//! programmer running `zdc check` is told.

/// Every message `zdc_types::check` reports for a source.
///
/// Through the public entry point and against the *real* placement pass
/// rather than a stand-in: §17.1.4's interface is what tells the integrity
/// pass which context a body runs in, and a hand-written answer here would
/// be a second copy of it. That is also why these live in a test binary
/// rather than beside the pass — the dependency on `zdc-graph` is a cycle,
/// and a development cycle is only sound one crate out.
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
         state typed is client Text starting \"\"\n\
         view\n\
         \x20   Input typed\n\
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
    // A `Text` key, because the place is a `Map of Text to …`: the
    // question here is provenance, and a key of the wrong type would be
    // refused by inference before integrity ever saw it.
    let found = errors(
        "trusted state moderators is durable Map of Text to Truth starting empty\n\
         state typed is client Text starting \"\"\n\
         view\n\
         \x20   Input typed\n\
         \x20       on keydown with press\n\
         \x20           set moderators at press.key to yes\n",
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

/// **The corpus reaches every code this pass can print.**
///
/// The counterpart to `zdc-diagnostics`'s explanation coverage: that test
/// asserts every code in this crate's source has a `zdc explain` entry
/// behind it, and this one asserts every such code has a *program* that
/// provokes it. A code with an explanation and no fixture is a rule
/// nothing has ever run.
///
/// The list of codes is read out of this file's own assertions rather than
/// written down twice, and out of the pass's source, so adding a code
/// without a fixture fails here and deleting a fixture does too.
#[test]
fn every_integrity_code_the_pass_can_print_has_a_fixture() {
    use std::collections::BTreeSet;

    let codes_in = |text: &str| -> BTreeSet<String> {
        let bytes: Vec<char> = text.chars().collect();
        let mut found = BTreeSet::new();
        for start in 0..bytes.len() {
            if start + 8 > bytes.len() {
                break;
            }
            let candidate: String = bytes[start..start + 8].iter().collect();
            if let Some(rest) = candidate.strip_prefix("E-INT-") {
                if rest.chars().all(|c| c.is_ascii_digit()) {
                    found.insert(candidate);
                }
            }
        }
        found
    };

    let pass = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/integrity.rs"),
    )
    .expect("the integrity pass is readable");
    let in_source = codes_in(&pass);
    assert!(
        in_source.len() >= 4,
        "the scan found only {} codes, which means it stopped working: {in_source:?}",
        in_source.len()
    );

    let fixtures = std::fs::read_to_string(file!())
        .or_else(|_| {
            std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/integrity.rs"),
            )
        })
        .expect("this file is readable");
    // Only the codes an `expected …` assertion names: a code mentioned in
    // a doc comment is not a fixture.
    let asserted: BTreeSet<String> = fixtures
        .lines()
        .filter(|line| line.contains("\"expected E-INT-"))
        .flat_map(codes_in)
        .collect();

    let missing: Vec<&String> = in_source.difference(&asserted).collect();
    assert!(
        missing.is_empty(),
        "these integrity codes have no fixture that provokes them: {missing:?}"
    );
}
