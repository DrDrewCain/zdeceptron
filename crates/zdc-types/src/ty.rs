//! Types, and the small closed set of built-in constraints.
//!
//! Spec §5.4 lists the types. It also says "no typeclasses in v1", which
//! settles what a constraint may be: not something a program can declare,
//! only one of the fixed operand sets the built-in operators already
//! imply. `+` accepts numbers or `Text` (§16.7 item 1); `add` accepts
//! numbers only (§14B.2); a view element shows a base type (§16.3.6); `at`
//! and `empty` accept a collection (§5.4). Those four sets are the whole
//! list, and no program can extend it.

use std::fmt;

/// A unification variable.
pub type TyVarId = u32;

/// A ZDeceptron type.
///
/// `Whole` and `Decimal` are distinct here even though both compile to
/// f64 (§14A.3, §16.7): the source language separates them, so mixing
/// them is a mistake the checker should name even when codegen would not
/// have noticed.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Text,
    Whole,
    Decimal,
    Truth,
    /// The payload of `Failed` (§14G.1.2 writes `Failed with error is
    /// Error`). The spec never says what `Error` is; every example reads
    /// `.message` from it and nothing else, so that is the one field this
    /// compiler knows about. See the report's spec-defect list.
    Error,
    /// A type name the program wrote that no declaration defines.
    ///
    /// `record` and `choice` (§14B.1) are specified but not implemented,
    /// so `Item`, `Player`, `Todo` and friends cannot be looked up. They
    /// are treated as distinct opaque types rather than as errors: a
    /// program that names a type the language will have is not wrong, it
    /// is early. Two different names are never interchangeable, so this
    /// still catches real mistakes.
    Named(String),
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Option(Box<Type>),
    Remote(Box<Type>),
    /// A top-level `function`. Not a value: ZDeceptron has no first-class
    /// functions, so this only ever appears as a callee's type.
    Function(Vec<Type>, Box<Type>),
    Var(TyVarId),
    /// The result of something already reported.
    ///
    /// It unifies with everything and constrains nothing, so one mistake
    /// produces one diagnostic instead of a cascade.
    Unknown,
}

impl Type {
    pub fn list(inner: Type) -> Type {
        Type::List(Box::new(inner))
    }

    pub fn map(key: Type, value: Type) -> Type {
        Type::Map(Box::new(key), Box::new(value))
    }

    pub fn option(inner: Type) -> Type {
        Type::Option(Box::new(inner))
    }

    pub fn remote(inner: Type) -> Type {
        Type::Remote(Box::new(inner))
    }

    pub fn function(params: Vec<Type>, result: Type) -> Type {
        Type::Function(params, Box::new(result))
    }

    /// Whether anything is still unresolved inside this type. A type that
    /// still holds a variable cannot answer a question codegen will ask.
    pub fn is_settled(&self) -> bool {
        match self {
            Type::Var(_) | Type::Unknown => false,
            Type::Text | Type::Whole | Type::Decimal | Type::Truth | Type::Error => true,
            Type::Named(_) => true,
            Type::List(inner) | Type::Option(inner) | Type::Remote(inner) => inner.is_settled(),
            Type::Map(key, value) => key.is_settled() && value.is_settled(),
            Type::Function(params, result) => {
                params.iter().all(Type::is_settled) && result.is_settled()
            }
        }
    }
}

/// How a type reads in a diagnostic.
///
/// No Rust type name may reach a user-facing message (§7.3), so this is
/// the only way a type is ever printed.
impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Text => write!(f, "Text"),
            Type::Whole => write!(f, "Whole"),
            Type::Decimal => write!(f, "Decimal"),
            Type::Truth => write!(f, "Truth"),
            Type::Error => write!(f, "Error"),
            Type::Named(name) => write!(f, "{name}"),
            Type::List(inner) => write!(f, "List of {inner}"),
            Type::Map(key, value) => write!(f, "Map of {key} to {value}"),
            Type::Option(inner) => write!(f, "Option of {inner}"),
            Type::Remote(inner) => write!(f, "Remote of {inner}"),
            Type::Function(params, result) => {
                write!(f, "a function of {} giving {result}", params.len())
            }
            Type::Var(_) | Type::Unknown => write!(f, "a type that is not known here"),
        }
    }
}

/// The operand set a variable is restricted to.
///
/// Not a typeclass: the list is closed, no program can add to it, and a
/// constrained variable is never generalised, so no scheme ever carries a
/// qualification. That is the same discipline OCaml applies to weak
/// variables and it keeps §5.4's "no typeclasses in v1" literally true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
    /// Any type at all.
    Any,
    /// `Text`, `Whole`, `Decimal`, `Truth` — what a view element can turn
    /// into a text node (§16.3.6).
    Shown,
    /// `Whole`, `Decimal`, `Text` — the operands `+` accepts (§16.7).
    Addable,
    /// `Whole`, `Decimal` — arithmetic and comparison (§14B.2).
    Numeric,
    /// `List of T`, `Map of K to V` — what `at` indexes and `empty`
    /// creates (§5.4).
    Collection,
}

impl Constraint {
    /// Whether a concrete type satisfies this constraint.
    pub fn admits(self, ty: &Type) -> bool {
        match self {
            Constraint::Any => true,
            Constraint::Shown => {
                matches!(ty, Type::Text | Type::Whole | Type::Decimal | Type::Truth)
            }
            Constraint::Addable => matches!(ty, Type::Text | Type::Whole | Type::Decimal),
            Constraint::Numeric => matches!(ty, Type::Whole | Type::Decimal),
            Constraint::Collection => matches!(ty, Type::List(_) | Type::Map(_, _)),
        }
    }

    /// The constraint satisfying both, or `None` when nothing does.
    ///
    /// The four value constraints form a chain — numbers ⊆ addable ⊆
    /// shown ⊆ anything — so their meet is whichever is narrower.
    /// `Collection` is disjoint from all three, which is exactly why
    /// `1 + xs` is rejected rather than silently deferred.
    pub fn meet(self, other: Constraint) -> Option<Constraint> {
        if self == other {
            return Some(self);
        }
        match (self, other) {
            (Constraint::Any, c) | (c, Constraint::Any) => Some(c),
            (Constraint::Collection, _) | (_, Constraint::Collection) => None,
            _ => Some(if self.rank() > other.rank() {
                self
            } else {
                other
            }),
        }
    }

    /// Position in the chain; higher is narrower.
    fn rank(self) -> u8 {
        match self {
            Constraint::Any => 0,
            Constraint::Shown => 1,
            Constraint::Addable => 2,
            Constraint::Numeric => 3,
            Constraint::Collection => 4,
        }
    }

    /// The type a variable still carrying this constraint becomes when
    /// nothing else pinned it down.
    ///
    /// Defaulting is what keeps `give 1 + 2` from needing an annotation.
    /// `Collection` has no default: a list and a map are not
    /// interchangeable, and guessing would pick the wrong runtime helper
    /// (§16.7 item 5).
    pub fn default_type(self) -> Option<Type> {
        match self {
            Constraint::Any | Constraint::Collection => None,
            Constraint::Shown => Some(Type::Text),
            Constraint::Addable | Constraint::Numeric => Some(Type::Whole),
        }
    }

    /// How the constraint reads in a diagnostic.
    pub fn describe(self) -> &'static str {
        match self {
            Constraint::Any => "any type",
            Constraint::Shown => "`Text`, `Whole`, `Decimal`, or `Truth`",
            Constraint::Addable => "`Whole`, `Decimal`, or `Text`",
            Constraint::Numeric => "`Whole` or `Decimal`",
            Constraint::Collection => "a `List` or a `Map`",
        }
    }

    /// What a value carrying this constraint *is*, for the diagnostic
    /// that has to name a literal whose exact type nothing pinned down.
    /// `1` is a number before it is a `Whole`.
    pub fn subject(self) -> &'static str {
        match self {
            Constraint::Any => "a value",
            Constraint::Shown => "a value shown as text",
            Constraint::Addable => "a number or text",
            Constraint::Numeric => "a number",
            Constraint::Collection => "a collection",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_type_prints_the_way_the_program_would_write_it() {
        assert_eq!(Type::list(Type::Text).to_string(), "List of Text");
        assert_eq!(
            Type::map(Type::Text, Type::Whole).to_string(),
            "Map of Text to Whole"
        );
        assert_eq!(
            Type::remote(Type::list(Type::Named("Item".into()))).to_string(),
            "Remote of List of Item"
        );
    }

    #[test]
    fn the_value_constraints_form_a_chain() {
        assert_eq!(
            Constraint::Numeric.meet(Constraint::Addable),
            Some(Constraint::Numeric)
        );
        assert_eq!(
            Constraint::Shown.meet(Constraint::Any),
            Some(Constraint::Shown)
        );
    }

    #[test]
    fn a_collection_never_meets_a_value_constraint() {
        assert_eq!(Constraint::Collection.meet(Constraint::Numeric), None);
        assert_eq!(Constraint::Shown.meet(Constraint::Collection), None);
    }

    #[test]
    fn shown_admits_every_base_type_and_nothing_else() {
        for ty in [Type::Text, Type::Whole, Type::Decimal, Type::Truth] {
            assert!(Constraint::Shown.admits(&ty), "{ty} should be showable");
        }
        assert!(!Constraint::Shown.admits(&Type::list(Type::Text)));
        assert!(!Constraint::Shown.admits(&Type::remote(Type::Text)));
    }
}
