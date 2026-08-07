//! The substitution and unification engine.
//!
//! Ordinary Hindley–Milner: a growable substitution over unification
//! variables, an occurs check, and structural unification everywhere
//! else. The two departures from the textbook are both spec-driven.
//!
//! * `Type::Unknown` unifies with anything and records nothing. It is the
//!   result of an error that has already been reported, and it exists so
//!   that one mistake produces one diagnostic rather than a cascade —
//!   the same rule name resolution follows.
//! * A variable may carry a `Constraint`. Binding it to a type the
//!   constraint does not admit fails, and unifying two variables takes
//!   the meet of theirs.

use crate::ty::{Constraint, TyVarId, Type};

/// Why two types could not be made equal.
///
/// The site that called `unify` owns the wording, because only it knows
/// whether the mismatch is an argument, a `set`, or a `when` arm. This
/// only says which of the two failure modes happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Mismatch {
    /// The two types are different shapes.
    Shape,
    /// A variable was asked to become a type its constraint forbids.
    Constraint { needed: Constraint, found: Type },
    /// A type would have had to contain itself.
    Infinite,
}

#[derive(Debug, Default)]
pub struct Solver {
    bindings: Vec<Option<Type>>,
    constraints: Vec<Constraint>,
}

impl Solver {
    pub fn new() -> Solver {
        Solver::default()
    }

    /// A fresh variable admitting anything.
    pub fn fresh(&mut self) -> Type {
        self.fresh_constrained(Constraint::Any)
    }

    pub fn fresh_constrained(&mut self, constraint: Constraint) -> Type {
        let id =
            TyVarId::try_from(self.bindings.len()).expect("a program has fewer than 2^32 types");
        self.bindings.push(None);
        self.constraints.push(constraint);
        Type::Var(id)
    }

    /// Follow bindings one level: the outermost shape of a type.
    pub fn shallow(&self, ty: &Type) -> Type {
        let mut current = ty.clone();
        while let Type::Var(id) = current {
            match &self.bindings[id as usize] {
                Some(bound) => current = bound.clone(),
                None => return Type::Var(id),
            }
        }
        current
    }

    /// Follow bindings everywhere: the fully resolved type.
    pub fn zonk(&self, ty: &Type) -> Type {
        match self.shallow(ty) {
            Type::List(inner) => Type::list(self.zonk(&inner)),
            Type::Option(inner) => Type::option(self.zonk(&inner)),
            Type::Remote(inner) => Type::remote(self.zonk(&inner)),
            Type::Map(key, value) => Type::map(self.zonk(&key), self.zonk(&value)),
            Type::Pair(first, second) => Type::pair(self.zonk(&first), self.zonk(&second)),
            Type::Function(params, result) => Type::function(
                params.iter().map(|param| self.zonk(param)).collect(),
                self.zonk(&result),
            ),
            settled => settled,
        }
    }

    pub fn constraint_of(&self, id: TyVarId) -> Constraint {
        self.constraints[id as usize]
    }

    /// Narrow a variable's constraint, or check a concrete type against
    /// one. Used where a constraint is imposed without an equation to go
    /// with it, such as an element's leading text slot.
    pub fn require(&mut self, ty: &Type, constraint: Constraint) -> Result<(), Mismatch> {
        match self.shallow(ty) {
            Type::Unknown => Ok(()),
            Type::Var(id) => match self.constraints[id as usize].meet(constraint) {
                Some(narrowed) => {
                    self.constraints[id as usize] = narrowed;
                    Ok(())
                }
                None => Err(Mismatch::Constraint {
                    needed: constraint,
                    found: Type::Var(id),
                }),
            },
            concrete if constraint.admits(&concrete) => Ok(()),
            concrete => Err(Mismatch::Constraint {
                needed: constraint,
                found: concrete,
            }),
        }
    }

    pub fn unify(&mut self, left: &Type, right: &Type) -> Result<(), Mismatch> {
        let left = self.shallow(left);
        let right = self.shallow(right);

        match (left, right) {
            // Already reported. Say nothing more about it.
            (Type::Unknown, _) | (_, Type::Unknown) => Ok(()),

            (Type::Var(a), Type::Var(b)) if a == b => Ok(()),
            (Type::Var(a), Type::Var(b)) => {
                let Some(narrowed) =
                    self.constraints[a as usize].meet(self.constraints[b as usize])
                else {
                    return Err(Mismatch::Constraint {
                        needed: self.constraints[a as usize],
                        found: Type::Var(b),
                    });
                };
                self.constraints[b as usize] = narrowed;
                self.bindings[a as usize] = Some(Type::Var(b));
                Ok(())
            }
            (Type::Var(id), concrete) | (concrete, Type::Var(id)) => self.bind(id, concrete),

            (Type::Text, Type::Text)
            // `Markup` unifies with itself and with nothing else — in
            // particular not with `Text`, which is the whole reason the
            // type exists (`Type::Markup`).
            | (Type::Markup, Type::Markup)
            | (Type::Whole, Type::Whole)
            | (Type::Decimal, Type::Decimal)
            | (Type::Truth, Type::Truth)
            | (Type::Error, Type::Error) => Ok(()),

            (Type::Event(a), Type::Event(b)) if a == b => Ok(()),

            (Type::Named(a), Type::Named(b)) if a == b => Ok(()),

            (Type::List(a), Type::List(b))
            | (Type::Option(a), Type::Option(b))
            | (Type::Remote(a), Type::Remote(b)) => self.unify(&a, &b),

            // Two operands each, matched in order. A pair does not unify
            // with a map of the same operands: `Map of Text to Whole` is a
            // collection of entries and `Pair of Text to Whole` is one, so
            // interchanging them would make `length of` answer about the
            // wrong thing.
            (Type::Map(ak, av), Type::Map(bk, bv)) | (Type::Pair(ak, av), Type::Pair(bk, bv)) => {
                self.unify(&ak, &bk)?;
                self.unify(&av, &bv)
            }

            (Type::Function(ap, ar), Type::Function(bp, br)) if ap.len() == bp.len() => {
                for (a, b) in ap.iter().zip(bp.iter()) {
                    self.unify(a, b)?;
                }
                self.unify(&ar, &br)
            }

            _ => Err(Mismatch::Shape),
        }
    }

    fn bind(&mut self, id: TyVarId, concrete: Type) -> Result<(), Mismatch> {
        if self.occurs(id, &concrete) {
            return Err(Mismatch::Infinite);
        }
        let constraint = self.constraints[id as usize];
        if !constraint.admits(&concrete) {
            return Err(Mismatch::Constraint {
                needed: constraint,
                found: concrete,
            });
        }
        self.bindings[id as usize] = Some(concrete);
        Ok(())
    }

    fn occurs(&self, id: TyVarId, ty: &Type) -> bool {
        match self.shallow(ty) {
            Type::Var(other) => other == id,
            Type::List(inner) | Type::Option(inner) | Type::Remote(inner) => {
                self.occurs(id, &inner)
            }
            Type::Map(key, value) | Type::Pair(key, value) => {
                self.occurs(id, &key) || self.occurs(id, &value)
            }
            Type::Function(params, result) => {
                params.iter().any(|param| self.occurs(id, param)) || self.occurs(id, &result)
            }
            _ => false,
        }
    }

    /// Every variable still free inside a type, in first-seen order.
    pub fn free_vars(&self, ty: &Type, out: &mut Vec<TyVarId>) {
        match self.shallow(ty) {
            Type::Var(id) => {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            Type::List(inner) | Type::Option(inner) | Type::Remote(inner) => {
                self.free_vars(&inner, out)
            }
            Type::Map(key, value) | Type::Pair(key, value) => {
                self.free_vars(&key, out);
                self.free_vars(&value, out);
            }
            Type::Function(params, result) => {
                for param in &params {
                    self.free_vars(param, out);
                }
                self.free_vars(&result, out);
            }
            _ => {}
        }
    }

    /// Replace every variable still carrying a default-able constraint
    /// with that default.
    ///
    /// Run once, after everything else. `give 1 + 2` leaves a numeric
    /// variable that nothing pinned down; §14A.3 says both numeric types
    /// are f64 anyway, so `Whole` is the answer that needs no annotation.
    pub fn default_unconstrained(&mut self) {
        for id in 0..self.bindings.len() {
            if self.bindings[id].is_some() {
                continue;
            }
            if let Some(default) = self.constraints[id].default_type() {
                self.bindings[id] = Some(default);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variable_takes_the_shape_it_is_unified_with() {
        let mut solver = Solver::new();
        let a = solver.fresh();
        solver.unify(&a, &Type::list(Type::Text)).expect("unifies");
        assert_eq!(solver.zonk(&a), Type::list(Type::Text));
    }

    #[test]
    fn two_shapes_that_differ_do_not_unify() {
        let mut solver = Solver::new();
        assert_eq!(
            solver.unify(&Type::Text, &Type::Whole),
            Err(Mismatch::Shape)
        );
    }

    #[test]
    fn unknown_absorbs_anything_so_one_mistake_is_one_diagnostic() {
        let mut solver = Solver::new();
        solver.unify(&Type::Unknown, &Type::Text).expect("absorbs");
        solver
            .unify(&Type::list(Type::Truth), &Type::Unknown)
            .expect("absorbs");
    }

    #[test]
    fn a_numeric_variable_refuses_text() {
        let mut solver = Solver::new();
        let n = solver.fresh_constrained(Constraint::Numeric);
        let result = solver.unify(&n, &Type::Text);
        assert!(matches!(result, Err(Mismatch::Constraint { .. })));
    }

    #[test]
    fn unifying_two_variables_keeps_the_narrower_constraint() {
        let mut solver = Solver::new();
        let numeric = solver.fresh_constrained(Constraint::Numeric);
        let anything = solver.fresh();
        solver.unify(&numeric, &anything).expect("unifies");
        assert!(matches!(
            solver.unify(&anything, &Type::Text),
            Err(Mismatch::Constraint { .. })
        ));
    }

    #[test]
    fn a_type_may_not_contain_itself() {
        let mut solver = Solver::new();
        let a = solver.fresh();
        assert_eq!(
            solver.unify(&a, &Type::list(a.clone())),
            Err(Mismatch::Infinite)
        );
    }

    #[test]
    fn an_undetermined_number_defaults_to_whole() {
        let mut solver = Solver::new();
        let n = solver.fresh_constrained(Constraint::Numeric);
        solver.default_unconstrained();
        assert_eq!(solver.zonk(&n), Type::Whole);
    }

    #[test]
    fn an_undetermined_collection_has_no_default() {
        let mut solver = Solver::new();
        let c = solver.fresh_constrained(Constraint::Collection);
        solver.default_unconstrained();
        assert!(matches!(solver.zonk(&c), Type::Var(_)));
    }
}
