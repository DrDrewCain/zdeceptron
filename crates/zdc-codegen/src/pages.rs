//! Per-URL specialisation: the address fold (spec §14G.2 revision 1).
//!
//! A routed program has one `view`, and that view dispatches on a signal
//! initialised `starting address`. That signal is **immutable** — the
//! browser writes it at load and the program never does — so its value is
//! a compile-time constant *for each emitted document*. Folding it is
//! therefore ordinary constant propagation over an immutable signal, not
//! a new evaluation mode, and everything that falls out of it falls out
//! for free:
//!
//! * **`when page` has one arm per document.** The other arms are not
//!   emitted, so `/blog`'s markup, handlers and helpers are not in
//!   `/work`'s bundle. This is what makes per-page splitting real rather
//!   than a claim: the split is a consequence of the fold, and §16.3.1's
//!   dead-code claim is discharged by the same closure walk that already
//!   keeps an unreferenced helper out.
//! * **A route parameter is a literal.** `/blog/rust` binds `slug` to
//!   `"rust"`, so a lookup over it constant-folds. §14C.3b's premise —
//!   "the value is present in the bundle" — is true of every value the
//!   instance can reach, and `/blog/rust` inlines one post rather than
//!   fifty-one.
//! * **The address signal itself disappears.** Nothing reads it after the
//!   fold, so the closure walk never reaches it and no cell is allocated
//!   for it.
//!
//! The fold rewrites nodes, never expressions. A binder it replaces is
//! recorded in [`Bindings`], which the emitter consults at the one place a
//! binder is read.

use std::collections::{BTreeMap, BTreeSet};

use zdc_hir::{
    DefId, DefKind, ExprId, Hir, HirExprKind, HirNode, HirNodeArmBody, LocalId, Res, RouteTable,
};
use zdc_types::Page;

use crate::expr::Literal;

/// What a binder was folded to.
#[derive(Debug, Clone)]
pub enum Binding {
    /// A route parameter's value in this document.
    Literal(Literal),
    /// A route value itself: the payload of `Some with here`.
    Route { variant: usize, values: Vec<String> },
}

/// Every binder the fold replaced with a constant.
#[derive(Debug, Clone, Default)]
pub struct Bindings {
    map: BTreeMap<LocalId, Binding>,
}

impl Bindings {
    pub fn get(&self, id: LocalId) -> Option<&Binding> {
        self.map.get(&id)
    }

    pub fn locals(&self) -> impl Iterator<Item = LocalId> + '_ {
        self.map.keys().copied()
    }
}

/// One page's nodes, after the fold.
pub struct Specialised {
    pub nodes: Vec<HirNode>,
    pub bindings: Bindings,
}

/// A value the fold knows.
#[derive(Debug, Clone, PartialEq)]
enum Folded {
    /// A variant with its fields in declaration order.
    Variant {
        name: String,
        fields: Vec<Folded>,
    },
    Text(String),
}

/// Specialise the view for one document.
pub fn specialise(hir: &Hir, nodes: &[HirNode], page: &Page) -> Specialised {
    let Some((route_def, table)) = &hir.routes else {
        return Specialised {
            nodes: nodes.to_vec(),
            bindings: Bindings::default(),
        };
    };

    let value = match page.variant {
        // `address` is `Option of <route>`: a URL the build wrote is
        // `Some`, and the not-found document is `None`. The `None` arm is
        // the not-found page, and exhaustiveness is what forced the
        // program to write it.
        Some(index) => Folded::Variant {
            name: "Some".to_string(),
            fields: vec![Folded::Variant {
                name: variant_name(hir, *route_def, index),
                fields: page.values.iter().cloned().map(Folded::Text).collect(),
            }],
        },
        None => Folded::Variant {
            name: "None".to_string(),
            fields: Vec::new(),
        },
    };

    let mut fold = Fold {
        hir,
        table,
        defs: address_signals(hir)
            .into_iter()
            .map(|def| (def, value.clone()))
            .collect(),
        locals: BTreeMap::new(),
        bindings: Bindings::default(),
    };
    let nodes = fold.nodes(nodes);
    Specialised {
        nodes,
        bindings: fold.bindings,
    }
}

/// Every signal initialised `starting address`.
///
/// There may be more than one, and they all hold the same value: the
/// signal is immutable, so two of them cannot disagree.
fn address_signals(hir: &Hir) -> BTreeSet<DefId> {
    let mut found = BTreeSet::new();
    for (id, def) in hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        if matches!(hir.exprs[signal.init].kind, HirExprKind::Address) {
            found.insert(id);
        }
    }
    found
}

fn variant_name(hir: &Hir, route: DefId, index: usize) -> String {
    let DefKind::Choice(choice) = &hir.defs[route].kind else {
        return String::new();
    };
    choice
        .variants
        .get(index)
        .map(|variant| variant.name.clone())
        .unwrap_or_default()
}

struct Fold<'a> {
    hir: &'a Hir,
    table: &'a RouteTable,
    defs: BTreeMap<DefId, Folded>,
    locals: BTreeMap<LocalId, Folded>,
    bindings: Bindings,
}

impl Fold<'_> {
    fn nodes(&mut self, nodes: &[HirNode]) -> Vec<HirNode> {
        let mut out = Vec::with_capacity(nodes.len());
        for node in nodes {
            match node {
                HirNode::When(when) => match self.value(when.scrutinee) {
                    // The scrutinee is known, so exactly one arm can run
                    // and the rest are not part of this document.
                    Some(Folded::Variant { name, fields }) => {
                        let Some(arm) = when.arms.iter().find(|arm| arm.pattern_name == name)
                        else {
                            continue;
                        };
                        self.bind(&arm.bindings, &fields);
                        let body = match &arm.body {
                            HirNodeArmBody::Show(element) => {
                                self.nodes(&[HirNode::Element((**element).clone())])
                            }
                            HirNodeArmBody::Nodes(nodes) => self.nodes(nodes),
                        };
                        out.extend(body);
                    }
                    // Not a value this document knows, so the `when`
                    // stays a `when` and every arm is emitted.
                    Some(Folded::Text(_)) | None => {
                        let mut when = when.clone();
                        for arm in &mut when.arms {
                            arm.body = match &arm.body {
                                HirNodeArmBody::Show(element) => {
                                    HirNodeArmBody::Show(element.clone())
                                }
                                HirNodeArmBody::Nodes(nodes) => {
                                    HirNodeArmBody::Nodes(self.nodes(nodes))
                                }
                            };
                        }
                        out.push(HirNode::When(when));
                    }
                },
                HirNode::Element(element) => {
                    let mut element = element.clone();
                    element.children = self.nodes(&element.children);
                    out.push(HirNode::Element(element));
                }
                HirNode::Each(each) => {
                    let mut each = each.clone();
                    each.body = self.nodes(&each.body);
                    out.push(HirNode::Each(each));
                }
                HirNode::If(conditional) => {
                    let mut conditional = conditional.clone();
                    conditional.then = self.nodes(&conditional.then);
                    conditional.otherwise = conditional
                        .otherwise
                        .as_ref()
                        .map(|nodes| self.nodes(nodes));
                    out.push(HirNode::If(conditional));
                }
                HirNode::Scope(scope) => {
                    let mut scope = scope.clone();
                    scope.body = self.nodes(&scope.body);
                    out.push(HirNode::Scope(scope));
                }
                HirNode::Handler(_) | HirNode::Children(_) => out.push(node.clone()),
            }
        }
        out
    }

    /// Bind an arm's binders to the fields of the value it matched.
    fn bind(&mut self, binders: &[LocalId], fields: &[Folded]) {
        for (binder, field) in binders.iter().zip(fields) {
            self.locals.insert(*binder, field.clone());
            match field {
                Folded::Text(text) => {
                    self.bindings
                        .map
                        .insert(*binder, Binding::Literal(Literal::Text(text.clone())));
                }
                Folded::Variant { name, fields } => {
                    // A route value bound whole, as `Some with here` does.
                    if let Some(variant) = self.variant_index(name) {
                        let values = fields
                            .iter()
                            .map(|field| match field {
                                Folded::Text(text) => text.clone(),
                                Folded::Variant { name, .. } => name.clone(),
                            })
                            .collect();
                        self.bindings
                            .map
                            .insert(*binder, Binding::Route { variant, values });
                    }
                }
            }
        }
    }

    fn variant_index(&self, name: &str) -> Option<usize> {
        let (def, _) = self.hir.routes.as_ref()?;
        let DefKind::Choice(choice) = &self.hir.defs[*def].kind else {
            return None;
        };
        let index = choice
            .variants
            .iter()
            .position(|variant| variant.name == name)?;
        self.table.variants.get(index).map(|_| index)
    }

    /// What an expression folds to, if anything.
    fn value(&self, id: ExprId) -> Option<Folded> {
        match &self.hir.exprs[id].kind {
            HirExprKind::Ref(Res::Def(def)) => self.defs.get(def).cloned(),
            HirExprKind::Ref(Res::Local(local)) => self.locals.get(local).cloned(),
            // `address` is folded at the signal that holds it, never at
            // the expression: §14G.2 revision 1 makes the *signal*
            // immutable, and that is what the fold is over.
            // A capability is answered by `evaluate`, and what it gave
            // reaches this fold as the `static` value it computed — never
            // as the capability expression itself.
            HirExprKind::Build { .. }
            | HirExprKind::Address
            | HirExprKind::Ref(Res::Builtin(_))
            | HirExprKind::Ref(Res::Variant { .. })
            | HirExprKind::Ref(Res::BuiltinVariant(_))
            | HirExprKind::OfCall { .. }
            | HirExprKind::Operator { .. }
            | HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty
            | HirExprKind::List(_)
            | HirExprKind::Map(_)
            | HirExprKind::Call { .. }
            | HirExprKind::Environment(_)
            | HirExprKind::Unary { .. }
            | HirExprKind::Binary { .. }
            | HirExprKind::Field { .. }
            | HirExprKind::Index { .. }
            | HirExprKind::Append { .. }
            | HirExprKind::Insert { .. } => None,
        }
    }
}
