//! The choice types `when` can eliminate, and the field list of each
//! variant.
//!
//! §14G.1.2: a variant carries *named* fields and a pattern binds fresh
//! names to them *positionally*. §16.7 item 4 asks for exactly this — the
//! choice type of every scrutinee plus its variants' declared field lists
//! in order — because `whenInto`'s `arm.length` contract cannot be
//! satisfied without it.
//!
//! Only the two built-in choices exist. `record` and `choice`
//! declarations (§14B.1) are specified but not implemented, so when they
//! land this is the one file that grows a third source of variants.

use crate::ty::Type;

/// One variant of a choice type: its tag and the types of its declared
/// fields, in declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: &'static str,
    pub fields: Vec<Type>,
}

/// The variants of a choice type, in declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// How the type reads in a diagnostic: `Remote of Text`.
    pub described: String,
    pub variants: Vec<Variant>,
}

impl Choice {
    pub fn variant(&self, name: &str) -> Option<&Variant> {
        self.variants.iter().find(|variant| variant.name == name)
    }

    /// `Loading`, `Ready`, and `Failed` — for the diagnostic that lists
    /// what an arm could have matched.
    pub fn variant_names(&self) -> String {
        let quoted: Vec<String> = self
            .variants
            .iter()
            .map(|variant| format!("`{}`", variant.name))
            .collect();
        match quoted.split_last() {
            None => String::new(),
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{}, and {last}", rest.join(", ")),
        }
    }
}

/// The choice a `when` scrutinee of this type eliminates, if it is one.
///
/// §14G.1.2 gives the built-ins field names for construction and
/// diagnostics: `Ready with value is T`, `Failed with error is Error`,
/// `Some with value is T`. `Loading` and `None` carry nothing.
pub fn choice_of(ty: &Type) -> Option<Choice> {
    match ty {
        Type::Remote(inner) => Some(Choice {
            described: ty.to_string(),
            variants: vec![
                Variant {
                    name: "Loading",
                    fields: Vec::new(),
                },
                Variant {
                    name: "Ready",
                    fields: vec![(**inner).clone()],
                },
                Variant {
                    name: "Failed",
                    fields: vec![Type::Error],
                },
            ],
        }),
        Type::Option(inner) => Some(Choice {
            described: ty.to_string(),
            variants: vec![
                Variant {
                    name: "Some",
                    fields: vec![(**inner).clone()],
                },
                Variant {
                    name: "None",
                    fields: Vec::new(),
                },
            ],
        }),
        _ => None,
    }
}

/// The fields of `Error`.
///
/// The spec names the type (§14G.1.2) and never defines it. Every example
/// reads `.message` and nothing else, so that is what this knows.
pub fn error_field(name: &str) -> Option<Type> {
    (name == "message").then_some(Type::Text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_has_the_three_arms_every_context_must_write() {
        let choice = choice_of(&Type::remote(Type::Text)).expect("a choice");
        let names: Vec<&str> = choice.variants.iter().map(|v| v.name).collect();
        assert_eq!(names, ["Loading", "Ready", "Failed"]);
    }

    #[test]
    fn readys_field_is_the_payload_and_faileds_is_an_error() {
        let choice = choice_of(&Type::remote(Type::list(Type::Whole))).expect("a choice");
        assert_eq!(
            choice.variant("Ready").expect("Ready").fields,
            [Type::list(Type::Whole)]
        );
        assert_eq!(
            choice.variant("Failed").expect("Failed").fields,
            [Type::Error]
        );
        assert!(choice
            .variant("Loading")
            .expect("Loading")
            .fields
            .is_empty());
    }

    #[test]
    fn option_has_two_variants() {
        let choice = choice_of(&Type::option(Type::Text)).expect("a choice");
        assert_eq!(choice.variant_names(), "`Some`, and `None`");
    }

    #[test]
    fn a_base_type_is_not_a_choice() {
        assert!(choice_of(&Type::Text).is_none());
        assert!(choice_of(&Type::list(Type::Text)).is_none());
    }
}
