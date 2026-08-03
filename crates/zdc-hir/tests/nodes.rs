use zdc_hir::{
    Builtin, DefId, Hir, HirArm, HirArmBody, HirExpr, HirExprKind, HirNodeArm, HirNodeArmBody, Res,
};
use zdc_lexer::Span;

#[test]
fn a_new_program_has_no_definitions_and_no_view() {
    let hir = Hir::new();

    assert!(hir.defs.is_empty());
    assert!(hir.locals.is_empty());
    assert!(hir.exprs.is_empty());
    assert!(hir.blocks.is_empty());
    assert_eq!(hir.view, None);
}

#[test]
fn a_resolved_reference_records_what_it_points_at() {
    let mut hir = Hir::new();
    let local = hir.locals.alloc(zdc_hir::Local {
        name: "item".to_string(),
        span: Span::new(0, 4),
    });

    assert!(matches!(Res::Local(local), Res::Local(_)));
    assert!(matches!(
        Res::Builtin(Builtin::Element(zdc_hir::BuiltinElement::Row)),
        Res::Builtin(Builtin::Element(_))
    ));
}

#[test]
fn an_expression_keeps_the_span_it_was_allocated_with() {
    let mut hir = Hir::new();
    let id = hir.exprs.alloc(HirExpr {
        kind: HirExprKind::Number(1.0),
        span: Span::new(0, 1),
    });

    assert_eq!(hir.exprs[id].span, Span::new(0, 1));
}

/// A pattern binds one name per named field of the matched variant, so a
/// binding site is a list rather than a single optional binder (spec
/// §14G.1.2). Both arm flavours must agree on that.
#[test]
fn an_arm_can_bind_several_names() {
    let mut hir = Hir::new();
    let why = hir.locals.alloc(zdc_hir::Local {
        name: "why".to_string(),
        span: Span::new(0, 3),
    });
    let moment = hir.locals.alloc(zdc_hir::Local {
        name: "moment".to_string(),
        span: Span::new(5, 11),
    });
    let body = hir.blocks.alloc(zdc_hir::HirBlock {
        stmts: Vec::new(),
        span: Span::new(0, 0),
    });

    let arm = HirArm {
        pattern_name: "Archived".to_string(),
        bindings: vec![why, moment],
        body: HirArmBody::Block(body),
        span: Span::new(0, 11),
    };
    let node_arm = HirNodeArm {
        pattern_name: "Archived".to_string(),
        bindings: vec![why, moment],
        body: HirNodeArmBody::Nodes(Vec::new()),
        span: Span::new(0, 11),
    };

    assert_eq!(arm.bindings.len(), 2);
    assert_eq!(node_arm.bindings, arm.bindings);
}

/// A definition ID addresses the definition arena and nothing else; the
/// view is one of those definitions rather than a parallel structure.
#[test]
fn the_view_is_recorded_as_a_definition_id() {
    let mut hir = Hir::new();
    let id: DefId = hir.defs.alloc(zdc_hir::Def {
        name: "view".to_string(),
        span: Span::new(0, 4),
        kind: zdc_hir::DefKind::View(zdc_hir::View {
            metadata: zdc_hir::Metadata::default(),
            nodes: Vec::new(),
        }),
    });
    hir.view = Some(id);

    assert_eq!(hir.view, Some(id));
    assert_eq!(hir.defs[id].name, "view");
}
