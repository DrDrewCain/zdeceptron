//! The information-flow lattice — spec §17.3.2.
//!
//! Three components, not one, and the third is an *addition* rather than a
//! consequence: §14G.1.3(d) gives a `Remote`'s `Failed` payload the join of
//! the **call's arguments**, which no join of shape and value computes and
//! which contradicts §14E.3's declassification of the success value.
//! Saying so plainly is the alternative to pretending three components fall
//! out of one invariant.

use std::collections::BTreeSet;

/// `Public ⊑ Secret`. §5.3's lattice is two-point, and the grammar has no
/// syntax to declare a public-shaped secret-valued store — which is what
/// makes §17.2.5 fatal 4's ruling determinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[must_use = "a secrecy level is a security obligation; dropping it is how a leak gets past the flow pass"]
pub enum Secrecy {
    #[default]
    Public,
    Secret,
}

impl Secrecy {
    pub fn join(self, other: Secrecy) -> Secrecy {
        if self == Secrecy::Secret || other == Secrecy::Secret {
            Secrecy::Secret
        } else {
            Secrecy::Public
        }
    }

    /// `self ⊑ other`.
    pub fn flows_to(self, other: Secrecy) -> bool {
        self <= other
    }

    pub fn describe(self) -> &'static str {
        match self {
            Secrecy::Public => "public",
            Secrecy::Secret => "secret",
        }
    }
}

/// What can be learned about a value, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Obs {
    /// Learnable without projecting: a collection's length and key set, a
    /// variant's tag, whether an `Option` is present, whether a store
    /// changed.
    Shape,
    /// What projecting, indexing or iterating yields.
    Value,
    /// The payload of `Remote`'s `Failed` arm.
    Failure,
}

impl Obs {
    pub const ALL: [Obs; 3] = [Obs::Shape, Obs::Value, Obs::Failure];
}

/// A concrete label. Invariant: `shape ⊑ value`.
///
/// That invariant is what makes §14G.1.3(b) fall out rather than be bolted
/// on: `keep`/`sort`/`take first` join onto `shape`, and the invariant then
/// drags `value` up, so a filtered list of public rows is secret in full;
/// `map each` joins only onto `value`, so a mapped list keeps a public
/// length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use = "a label is a security obligation; dropping it is how a leak gets past the flow pass"]
pub struct Label {
    pub shape: Secrecy,
    pub value: Secrecy,
    pub failure: Secrecy,
}

impl Label {
    pub fn scalar(secrecy: Secrecy) -> Label {
        Label {
            shape: secrecy,
            value: secrecy,
            failure: secrecy,
        }
    }

    pub fn get(self, obs: Obs) -> Secrecy {
        match obs {
            Obs::Shape => self.shape,
            Obs::Value => self.value,
            Obs::Failure => self.failure,
        }
    }
}

/// A symbolic label: `floor ⊔ ⨆_{p ∈ deps} label(arg_p)`.
///
/// **No witness field.** Verified non-terminating otherwise: `floor` and
/// `deps` are fixed from round 2 of a recursive function's fixpoint, but a
/// witness would grow by two steps every round and `Sym::eq` would report a
/// change forever. Witnesses are reconstructed after convergence (§17.3.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[must_use = "a symbolic label is a security obligation; dropping it is how a leak gets past the flow pass"]
pub struct Sym {
    pub floor: Secrecy,
    /// `(parameter index, which observation of it)`.
    pub deps: BTreeSet<(u32, Obs)>,
}

impl Sym {
    pub fn bottom() -> Sym {
        Sym::default()
    }

    pub fn floor(secrecy: Secrecy) -> Sym {
        Sym {
            floor: secrecy,
            deps: BTreeSet::new(),
        }
    }

    pub fn dep(param: u32, obs: Obs) -> Sym {
        Sym {
            floor: Secrecy::Public,
            deps: BTreeSet::from([(param, obs)]),
        }
    }

    /// The only operation there is. No meet, no complement, nothing
    /// branches on a label — which is what keeps every computed label in
    /// the normal form and makes a summary linear in arity.
    pub fn join(&self, other: &Sym) -> Sym {
        Sym {
            floor: self.floor.join(other.floor),
            deps: self.deps.union(&other.deps).copied().collect(),
        }
    }

    pub fn join_in_place(&mut self, other: &Sym) {
        self.floor = self.floor.join(other.floor);
        self.deps.extend(other.deps.iter().copied());
    }

    /// Substitute actual arguments for parameters. The result is a `Sym`
    /// in the caller's own parameter space, which is what makes summaries
    /// compose without a second fixpoint.
    pub fn instantiate(&self, args: &[SymLabel]) -> Sym {
        let mut out = Sym::floor(self.floor);
        for (index, obs) in &self.deps {
            if let Some(arg) = args.get(*index as usize) {
                out.join_in_place(arg.get(*obs));
            } else {
                // A call with too few arguments is a resolution or type
                // error reported elsewhere; assuming the worst here is the
                // safe direction for a may-analysis.
                out.floor = Secrecy::Secret;
            }
        }
        out
    }

    /// A `Sym` with no free parameters is already a verdict.
    pub fn concrete(&self) -> Secrecy {
        self.floor
    }

    pub fn is_bottom(&self) -> bool {
        self.floor == Secrecy::Public && self.deps.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[must_use = "a symbolic label is a security obligation; dropping it is how a leak gets past the flow pass"]
pub struct SymLabel {
    pub shape: Sym,
    pub value: Sym,
    pub failure: Sym,
}

impl SymLabel {
    pub fn bottom() -> SymLabel {
        SymLabel::default()
    }

    /// A label whose three components are the same symbol.
    pub fn triple(sym: Sym) -> SymLabel {
        SymLabel {
            shape: sym.clone(),
            value: sym.clone(),
            failure: sym,
        }
    }

    pub fn declared(label: Label) -> SymLabel {
        SymLabel {
            shape: Sym::floor(label.shape),
            value: Sym::floor(label.value),
            failure: Sym::floor(label.failure),
        }
    }

    pub fn get(&self, obs: Obs) -> &Sym {
        match obs {
            Obs::Shape => &self.shape,
            Obs::Value => &self.value,
            Obs::Failure => &self.failure,
        }
    }

    pub fn join(&self, other: &SymLabel) -> SymLabel {
        SymLabel {
            shape: self.shape.join(&other.shape),
            value: self.value.join(&other.value),
            failure: self.failure.join(&other.failure),
        }
    }

    pub fn join_in_place(&mut self, other: &SymLabel) {
        self.shape.join_in_place(&other.shape);
        self.value.join_in_place(&other.value);
        self.failure.join_in_place(&other.failure);
    }

    /// Join one symbol into all three components — what `⊔ pc` means.
    pub fn join_all(&mut self, sym: &Sym) {
        self.shape.join_in_place(sym);
        self.value.join_in_place(sym);
        self.failure.join_in_place(sym);
    }

    pub fn instantiate(&self, args: &[SymLabel]) -> SymLabel {
        SymLabel {
            shape: self.shape.instantiate(args),
            value: self.value.instantiate(args),
            failure: self.failure.instantiate(args),
        }
    }

    /// Restore `shape ⊑ value` after a rule that raised `shape`.
    pub fn settle(&mut self) {
        let shape = self.shape.clone();
        self.value.join_in_place(&shape);
    }

    pub fn concrete(&self) -> Label {
        Label {
            shape: self.shape.concrete(),
            value: self.value.concrete(),
            failure: self.failure.concrete(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lattice_is_two_point_and_join_is_maximum() {
        assert_eq!(Secrecy::Public.join(Secrecy::Public), Secrecy::Public);
        assert_eq!(Secrecy::Public.join(Secrecy::Secret), Secrecy::Secret);
        assert!(Secrecy::Public.flows_to(Secrecy::Secret));
        assert!(!Secrecy::Secret.flows_to(Secrecy::Public));
    }

    #[test]
    fn instantiating_a_summary_substitutes_argument_labels() {
        // `floor ⊔ ⨆deps` is the whole shape of a summary, so a function
        // that ignores its second argument cannot be tainted by it — which
        // is what keeps `politeGreeting`'s unused `key` out of the result.
        let uses_only_the_first = Sym::dep(0, Obs::Value);
        let args = vec![
            SymLabel::declared(Label::scalar(Secrecy::Public)),
            SymLabel::declared(Label::scalar(Secrecy::Secret)),
        ];
        assert_eq!(
            uses_only_the_first.instantiate(&args).concrete(),
            Secrecy::Public
        );

        let uses_the_second = Sym::dep(1, Obs::Value);
        assert_eq!(
            uses_the_second.instantiate(&args).concrete(),
            Secrecy::Secret
        );
    }

    #[test]
    fn a_raised_shape_drags_the_value_up() {
        // §17.3.2's invariant, which is what rejects `keep each v where
        // <secret predicate>` returning a "public" list of public rows.
        let mut label = SymLabel::bottom();
        label.shape = Sym::floor(Secrecy::Secret);
        label.settle();
        assert_eq!(label.value.concrete(), Secrecy::Secret);
    }
}
