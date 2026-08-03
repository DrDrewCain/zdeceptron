mod support;

use support::{def_named, verdict, GUESTBOOK};
use zdc_graph::label::{Sym, SymLabel};
use zdc_graph::{GraphError, Obs, Secrecy, Severity, Sink, SinkSite};
use zdc_lexer::Span;

#[test]
fn graph_error_builders_preserve_ordered_context() {
    let primary = Span::new(20, 24);
    let first = Span::new(2, 5);
    let second = Span::new(11, 17);

    let diagnostic = GraphError::new("E-test", "the value escaped", primary)
        .with_notes(vec![
            (first, "declared secret here".to_string()),
            (second, "then returned here".to_string()),
        ])
        .with_help("keep the value on the server");

    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.rendered_message(),
        "error[E-test]: the value escaped"
    );
    assert_eq!(diagnostic.span, primary);
    assert_eq!(diagnostic.notes[0].0, first);
    assert_eq!(diagnostic.notes[1].0, second);
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("keep the value on the server")
    );
}

#[test]
fn warning_diagnostics_are_not_returned_as_errors() {
    let diagnostic = GraphError::warning("W-test", "unused root", Span::new(4, 8));

    assert!(!diagnostic.is_error());
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        diagnostic.rendered_message(),
        "warning[W-test]: unused root"
    );
}

#[test]
fn a_summary_with_a_missing_argument_fails_closed() {
    let summary = Sym::dep(3, Obs::Value);

    assert_eq!(summary.instantiate(&[]).concrete(), Secrecy::Secret);
}

#[test]
fn settling_shape_dependencies_preserves_observation_boundaries() {
    let mut label = SymLabel::bottom();
    label.shape = Sym::dep(0, Obs::Shape);
    label.failure = Sym::dep(1, Obs::Failure);

    label.settle();

    assert!(label.value.deps.contains(&(0, Obs::Shape)));
    assert!(label.failure.deps.contains(&(1, Obs::Failure)));
    assert!(!label.failure.deps.contains(&(0, Obs::Shape)));
}

#[test]
fn clearance_is_scoped_to_both_sink_and_site() {
    let (hir, _, verdict) = verdict(GUESTBOOK);
    let visits = def_named(&hir, "visits");
    let site = SinkSite::LiveSync(visits);

    assert!(verdict.cleared(Sink::LiveSync, site).is_some());
    assert!(
        verdict.cleared(Sink::View, site).is_none(),
        "a clearance for live sync must not authorize a different sink"
    );
}
