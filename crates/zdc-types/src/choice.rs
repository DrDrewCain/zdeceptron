//! The choice types `when` can eliminate, and the field list of each
//! variant.
//!
//! §14G.1.2: a variant carries *named* fields and a pattern binds fresh
//! names to them *positionally*. §16.7 item 4 asks for exactly this — the
//! choice type of every scrutinee plus its variants' declared field lists
//! in order — because `whenInto`'s `arm.length` contract cannot be
//! satisfied without it.
//!
//! Two sources of variants: the built-in `Option`, `Remote` and `Code`,
//! below, and the `choice` declarations a program writes, which
//! [`crate::infer`] collects out of the HIR. Both produce the same
//! [`Choice`], so every rule about arms, arity and exhaustiveness is
//! written once.

use crate::failure::FailureCode;
use crate::ty::Type;

/// One variant of a choice type: its tag, the names of its declared fields
/// and their types, both in declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    /// Field names, for construction and for diagnostics. §14G.1.2 gives
    /// the built-ins names for exactly this reason.
    pub field_names: Vec<String>,
    pub fields: Vec<Type>,
}

impl Variant {
    fn payload_free(name: &str) -> Variant {
        Variant {
            name: name.to_string(),
            field_names: Vec::new(),
            fields: Vec::new(),
        }
    }

    fn one(name: &str, field: &str, ty: Type) -> Variant {
        Variant {
            name: name.to_string(),
            field_names: vec![field.to_string()],
            fields: vec![ty],
        }
    }
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

/// The choice a `when` scrutinee of this *built-in* type eliminates.
///
/// A user-declared `choice` is a [`Type::Named`], which this cannot answer
/// on its own — the declaration lives in the HIR. `Checker::choice_of`
/// consults both.
///
/// §14G.1.2 gives the built-ins field names for construction and
/// diagnostics: `Ready with value is T`, `Failed with error is Error`,
/// `Some with value is T`. `Loading` and `None` carry nothing, and neither
/// does any arm of `Code`.
pub fn builtin_choice_of(ty: &Type) -> Option<Choice> {
    match ty {
        Type::Remote(inner) => Some(Choice {
            described: ty.to_string(),
            variants: vec![
                Variant::payload_free("Loading"),
                Variant::one("Ready", "value", (**inner).clone()),
                Variant::one("Failed", "error", Type::Error),
            ],
        }),
        Type::Option(inner) => Some(Choice {
            described: ty.to_string(),
            variants: vec![
                Variant::one("Some", "value", (**inner).clone()),
                Variant::payload_free("None"),
            ],
        }),
        Type::Code => Some(code_choice()),
        _ => None,
    }
}

/// The arms of `Code`, built from [`FailureCode`] rather than restated.
///
/// This is the only place the surface variants come from, so a fourth
/// [`FailureCode`] appears here — and therefore in every `when`'s
/// exhaustiveness check, in the resolver's variant table, and in the
/// diagnostic that lists the arms — without any of them being edited. The
/// spellings are the same ones `runtime/rpc.js` writes, which is what the
/// pinning test in `zdc-codegen` compares.
pub fn code_choice() -> Choice {
    Choice {
        described: Type::Code.to_string(),
        variants: FailureCode::CLOSED_SET
            .iter()
            .map(|code| Variant::payload_free(code.spelling()))
            .collect(),
    }
}

/// The one field of `Error` the client runtime writes from its own
/// control flow rather than from the response.
///
/// Named once, here, because two passes have to agree about it: the
/// checker types it, and `zdc-graph`'s flow pass gives it `public` where
/// every other field of every other record inherits the record's label.
/// Two spellings of it would be a soundness hole in one direction and a
/// dead exception in the other.
pub const ERROR_CODE_FIELD: &str = "code";

/// The fields of `Error`, in declaration order.
///
/// The spec names the type (§14G.1.2) and never defines it. Two fields,
/// at two labels, and the split between them is the whole point:
///
/// - `message` is host text and carries §14G.1.3(d)'s join, so it is as
///   secret as whatever the endpoint read.
/// - `code` is written by the client runtime from the transport outcome
///   and never from a byte the server sent, so it is `public` by
///   construction. See [`crate::failure`] for the closed set of values it
///   can hold and for the candidate that was dropped.
///
/// `message` is `Text`; `code` is [`Type::Code`], the built-in choice
/// whose arms are exactly that closed set. **Changing `code`'s type does
/// not change its label.** The flow pass keys its one exception to §17.6
/// item 15's field-insensitivity on the field *name* and on the binder
/// having come from a `Failed` pattern, and neither of those moved — so
/// `error.code` stays public and `error.message` stays worth whatever the
/// endpoint read.
pub fn error_fields() -> [(&'static str, Type); 2] {
    [("message", Type::Text), (ERROR_CODE_FIELD, Type::Code)]
}

/// The type of one field of `Error`, or `None` if it has no such field.
pub fn error_field(name: &str) -> Option<Type> {
    error_fields()
        .into_iter()
        .find(|(field, _)| *field == name)
        .map(|(_, ty)| ty)
}

/// The field names, quoted and joined, for the diagnostic that lists them.
pub fn error_field_names() -> String {
    let quoted: Vec<String> = error_fields()
        .iter()
        .map(|(field, _)| format!("`{field}`"))
        .collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_has_the_three_arms_every_context_must_write() {
        let choice = builtin_choice_of(&Type::remote(Type::Text)).expect("a choice");
        let names: Vec<&str> = choice
            .variants
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Loading", "Ready", "Failed"]);
    }

    #[test]
    fn readys_field_is_the_payload_and_faileds_is_an_error() {
        let choice = builtin_choice_of(&Type::remote(Type::list(Type::Whole))).expect("a choice");
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

    /// §14G.1.2: the built-ins carry field names too, so `Ready with value
    /// is T` can be named in a diagnostic.
    #[test]
    fn the_builtins_carry_the_field_names_14g12_gives_them() {
        let choice = builtin_choice_of(&Type::option(Type::Text)).expect("a choice");
        assert_eq!(choice.variant("Some").expect("Some").field_names, ["value"]);
        let remote = builtin_choice_of(&Type::remote(Type::Text)).expect("a choice");
        assert_eq!(
            remote.variant("Failed").expect("Failed").field_names,
            ["error"]
        );
    }

    #[test]
    fn option_has_two_variants() {
        let choice = builtin_choice_of(&Type::option(Type::Text)).expect("a choice");
        assert_eq!(choice.variant_names(), "`Some`, and `None`");
    }

    #[test]
    fn a_base_type_is_not_a_builtin_choice() {
        assert!(builtin_choice_of(&Type::Text).is_none());
        assert!(builtin_choice_of(&Type::list(Type::Text)).is_none());
        assert!(builtin_choice_of(&Type::Named("Status".into())).is_none());
    }

    /// `Code` is a choice `when` eliminates, and its arms are the closed
    /// set — named here in the test's own text, so dropping one stops
    /// this compiling and adding one leaves it unmentioned.
    #[test]
    fn code_is_a_builtin_choice_of_the_three_transport_outcomes() {
        let choice = builtin_choice_of(&Type::Code).expect("a choice");
        let names: Vec<&str> = choice.variants.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["Unreachable", "Timeout", "Rejected"]);
        assert_eq!(
            choice.variant_names(),
            "`Unreachable`, `Timeout`, and `Rejected`"
        );
    }

    /// No arm of `Code` carries anything, so `Timeout` is a value and
    /// never a constructor, and a pattern over it binds no names.
    #[test]
    fn every_code_arm_is_payload_free() {
        let choice = builtin_choice_of(&Type::Code).expect("a choice");
        let mut checked = 0;
        for variant in &choice.variants {
            assert!(
                variant.fields.is_empty(),
                "{} carries a payload",
                variant.name
            );
            assert!(variant.field_names.is_empty());
            checked += 1;
        }
        assert_eq!(checked, FailureCode::CLOSED_SET.len(), "an arm was skipped");
    }

    /// The arms come from [`FailureCode`] and are not restated, so the
    /// surface language and the compiler's own set cannot drift.
    #[test]
    fn the_arms_of_code_are_the_failure_codes_themselves() {
        let choice = builtin_choice_of(&Type::Code).expect("a choice");
        let arms: Vec<&str> = choice.variants.iter().map(|v| v.name.as_str()).collect();
        let codes: Vec<&str> = FailureCode::CLOSED_SET
            .iter()
            .map(|code| code.spelling())
            .collect();
        assert_eq!(arms, codes);
    }

    /// The type of `code` moved; the field list did not.
    #[test]
    fn the_error_record_still_has_two_fields_and_code_is_the_choice() {
        assert_eq!(error_field("message"), Some(Type::Text));
        assert_eq!(error_field(ERROR_CODE_FIELD), Some(Type::Code));
        assert_eq!(error_field("status"), None);
        assert_eq!(error_field_names(), "`message` and `code`");
    }
}
