//! Which functions call one another in tail position, and back again.
//!
//! `stmt.rs` rewrites a function that gives the result of calling *itself*
//! into a loop, so `sumFrom` folds a million elements in one frame. That
//! rewrite fires on a self-call and nothing else, and the gap it leaves is
//! the one #198 records: two functions that give the result of calling each
//! other are as much a loop as one that calls itself, and neither of them
//! is a self-call, so both stayed recursion. `examples/sorting.zd` measured
//! the cost — a merge written across two functions dies at 3200 elements
//! where the one-function spelling merges a hundred thousand.
//!
//! # Why this is a group and not a pair
//!
//! `f` calls `g` calls `h` calls `f` is the same shape as `f` calls `g`
//! calls `f`, and a rule written for pairs would take the second and miss
//! the first for no reason a programmer could predict. So the unit here is
//! the *cycle*: every function that can reach itself through tail calls,
//! together with everything on the way round.
//!
//! That is a strongly connected component of the tail-call graph. It is
//! computed by transitive closure rather than by Tarjan, which is the
//! slower algorithm and the one `analysis.rs::solve_reactive_functions`
//! already uses for the same kind of question — a fixed point over a
//! relation, iterated until nothing changes. A program's function count is
//! in the hundreds; the constant factor is not the thing to optimise, and
//! matching the file next door is worth more than the asymptotics.
//!
//! # What a group does *not* include
//!
//! A function whose only cycle is with itself. `continue $tail` already
//! handles that, it costs no allocation, and routing it through a
//! trampoline instead would make the common case slower to fix the rare
//! one. A self-call inside a group member still becomes `continue $tail`;
//! only the calls that cross to another member bounce.

use std::collections::{BTreeMap, BTreeSet};

use zdc_hir::{BlockId, DefId, DefKind, ExprId, Hir, HirArmBody, HirExprKind, HirStmt, Res};

/// The mutual tail-recursion cycles in one program.
#[derive(Debug, Default)]
pub struct TailGroups {
    /// Every function that is in a cycle of length greater than one,
    /// mapped to the whole cycle it belongs to — itself included.
    groups: BTreeMap<DefId, BTreeSet<DefId>>,
}

impl TailGroups {
    /// The cycle `def` belongs to, or nothing if it is in none.
    ///
    /// A function in no cycle is emitted exactly as it was before this
    /// module existed, which is what keeps §16.4's worked output
    /// byte-identical for every program that does not need the rewrite.
    pub fn group_of(&self, def: DefId) -> Option<&BTreeSet<DefId>> {
        self.groups.get(&def)
    }

    /// Find them.
    pub fn find(hir: &Hir) -> Self {
        // One step of the relation: who this function gives the result of
        // calling.
        let mut reach: BTreeMap<DefId, BTreeSet<DefId>> = BTreeMap::new();
        for (id, def) in hir.defs.iter() {
            let DefKind::Function(function) = &def.kind else {
                continue;
            };
            let mut callees = BTreeSet::new();
            tail_callees(hir, function.body, &mut callees);
            reach.insert(id, callees);
        }

        // Transitive closure. Each round adds the callees of everything
        // already reachable; when a round adds nothing the relation is
        // closed. This terminates because the sets only grow and the
        // number of functions is finite.
        let ids: Vec<DefId> = reach.keys().copied().collect();
        loop {
            let mut changed = false;
            for id in &ids {
                let mut grown = reach[id].clone();
                for step in &reach[id] {
                    if let Some(further) = reach.get(step) {
                        grown.extend(further.iter().copied());
                    }
                }
                if grown.len() != reach[id].len() {
                    reach.insert(*id, grown);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        // Two functions are in one cycle exactly when each reaches the
        // other. A function that reaches only itself is a self-call and
        // is deliberately not a group: `continue $tail` is already the
        // better answer for it.
        let mut groups = BTreeMap::new();
        for id in &ids {
            let others: BTreeSet<DefId> = reach[id]
                .iter()
                .copied()
                .filter(|other| {
                    other != id && reach.get(other).is_some_and(|back| back.contains(id))
                })
                .collect();
            if others.is_empty() {
                continue;
            }
            let mut whole = others;
            whole.insert(*id);
            groups.insert(*id, whole);
        }
        Self { groups }
    }
}

/// Every function this block gives the result of calling, in tail
/// position.
///
/// The shape of the walk is `stmt::gives_a_self_call`'s, and for the same
/// reason: a `give` is in tail position, an arm's expression is a `give`,
/// and everything else in a body has work left to do after the call comes
/// back.
fn tail_callees(hir: &Hir, block: BlockId, out: &mut BTreeSet<DefId>) {
    for stmt in &hir.blocks[block].stmts {
        match stmt {
            HirStmt::Give(expr) => insert_callee(hir, *expr, out),
            HirStmt::When(when) => {
                for arm in &when.arms {
                    match arm.body {
                        HirArmBody::Show(expr) => insert_callee(hir, expr, out),
                        HirArmBody::Block(body) => tail_callees(hir, body, out),
                    }
                }
            }
            HirStmt::Each(each) => tail_callees(hir, each.body, out),
            HirStmt::If(conditional) => {
                tail_callees(hir, conditional.then, out);
                if let Some(otherwise) = conditional.otherwise {
                    tail_callees(hir, otherwise, out);
                }
            }
            // None of these can be in tail position: a `do` gives nothing,
            // and the other three give nothing back either.
            HirStmt::Pipeline(_) | HirStmt::Mutation(_) | HirStmt::Bind(_) | HirStmt::Do(_) => {}
        }
    }
}

fn insert_callee(hir: &Hir, expr: ExprId, out: &mut BTreeSet<DefId>) {
    if let Some(def) = called_def(hir, expr) {
        out.insert(def);
    }
}

/// The function `expr` calls, if `expr` is a call and nothing wrapped
/// around it.
///
/// `1 + (f with …)` is not one: the addition still has to happen after the
/// call returns, so the frame cannot be reused.
pub fn called_def(hir: &Hir, expr: ExprId) -> Option<DefId> {
    let callee = match &hir.exprs[expr].kind {
        HirExprKind::Call { callee, .. } | HirExprKind::OfCall { callee, .. } => callee,
        _ => return None,
    };
    match callee {
        Res::Def(def) => Some(*def),
        _ => None,
    }
}

/// The name the inner function of a group member is emitted under.
///
/// The member keeps its own name for the wrapper, so every call site in
/// the program — and every other bundle — goes on naming what it always
/// named. `$` is not a character a ZDeceptron name can contain, so this
/// cannot collide with a program's own function however it is spelled.
pub fn step_name(name: &str) -> String {
    format!("$step${name}")
}
