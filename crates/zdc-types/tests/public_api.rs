use zdc_hir::{DefId, ExprId, Hir};
use zdc_types::{
    check, read_kind, Constraint, Placements, ReadContext, ReadKind, SignalPlacement, Type,
};

fn hir(source: &str) -> Hir {
    let program = zdc_parser::parse(source).expect("the fixture parses");
    zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect("the fixture resolves")
}

struct UniformPlacements(ReadKind);

impl Placements for UniformPlacements {
    fn read_contexts(&self, _def: DefId) -> Vec<ReadContext> {
        vec![ReadContext::Client]
    }

    fn read_kind_at(&self, _expr: ExprId, _context: ReadContext) -> ReadKind {
        self.0.clone()
    }
}

#[test]
fn placement_answers_control_the_type_of_every_signal_read() {
    let program = hir("state count is client Whole starting 0\n\
         state doubled is client Whole from count + count\n\
         view\n    Text doubled\n");

    assert!(check(&program, &UniformPlacements(ReadKind::Direct)).is_ok());

    let errors = check(&program, &UniformPlacements(ReadKind::Remote))
        .expect_err("remote reads cannot be used as plain Whole values");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("Remote of Whole")),
        "{errors:#?}"
    );
}

#[test]
fn forbidden_placement_answers_become_actionable_type_errors() {
    let program = hir("state count is client Whole starting 0\n\
         state doubled is client Whole from count\n");
    let reason = "this test context has no access to that state";

    let errors = check(&program, &UniformPlacements(ReadKind::Forbidden(reason)))
        .expect_err("a forbidden read must reject the program");

    assert!(errors.iter().any(|error| {
        error.message.contains("`count` is `client` state") && error.message.contains(reason)
    }));
}

#[test]
fn the_public_read_table_is_total_over_every_context_and_placement() {
    let contexts = [
        ReadContext::Client,
        ReadContext::Static,
        ReadContext::ViewRootedServer,
        ReadContext::TriggerRootedServer,
    ];
    let placements = [
        SignalPlacement::Client,
        SignalPlacement::Static,
        SignalPlacement::Server,
        SignalPlacement::Durable,
        SignalPlacement::DurablePerVisitor,
    ];

    for context in contexts {
        assert!(!context.describe().is_empty());
        for placement in placements {
            assert!(!placement.describe().is_empty());
            if let ReadKind::Forbidden(reason) = read_kind(context, placement) {
                assert!(!reason.is_empty());
            }
        }
    }
}

#[test]
fn type_constructors_preserve_nested_shape_and_settlement() {
    let settled = Type::remote(Type::option(Type::map(
        Type::Text,
        Type::list(Type::Named("Item".to_string())),
    )));
    assert_eq!(
        settled.to_string(),
        "Remote of Option of Map of Text to List of Item"
    );
    assert!(settled.is_settled());

    let unsettled = Type::function(
        vec![Type::Whole, Type::list(Type::Var(7))],
        Type::option(Type::Truth),
    );
    assert!(!unsettled.is_settled());
}

#[test]
fn builtin_type_names_round_trip_through_the_public_parser() {
    assert_eq!(
        Type::builtin_names(),
        ["Text", "Whole", "Decimal", "Truth", "Error"]
    );
    for name in Type::builtin_names() {
        let parsed = Type::from_name(name);
        assert!(Type::is_builtin_name(name));
        assert_eq!(parsed.to_string(), *name);
    }

    assert_eq!(
        Type::from_name("Customer"),
        Type::Named("Customer".to_string())
    );
    assert!(!Type::is_builtin_name("Customer"));
}

#[test]
fn constraint_meet_is_commutative_and_matches_set_intersection() {
    let constraints = [
        Constraint::Any,
        Constraint::Shown,
        Constraint::Addable,
        Constraint::Numeric,
        Constraint::Collection,
    ];
    let representatives = [
        Type::Text,
        Type::Whole,
        Type::Decimal,
        Type::Truth,
        Type::Error,
        Type::list(Type::Text),
        Type::map(Type::Text, Type::Whole),
        Type::remote(Type::Text),
    ];

    for left in constraints {
        assert_eq!(left.meet(left), Some(left));
        assert!(!left.describe().is_empty());
        assert!(!left.subject().is_empty());

        for right in constraints {
            let meet = left.meet(right);
            assert_eq!(meet, right.meet(left));
            for ty in &representatives {
                let intersection = left.admits(ty) && right.admits(ty);
                assert_eq!(
                    meet.is_some_and(|constraint| constraint.admits(ty)),
                    intersection,
                    "{left:?} ∩ {right:?} disagrees for {ty}"
                );
            }
        }
    }
}
