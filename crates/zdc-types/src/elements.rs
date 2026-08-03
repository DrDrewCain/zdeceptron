//! What each built-in view element accepts, per §16.3.6.
//!
//! `zdc-codegen` owns the same table's *DOM* half — tag, attributes, base
//! class. This owns its *type* half, which is the half codegen cannot
//! check. The two are kept apart deliberately: codegen's table says what
//! markup an element becomes, and drifting from `elements.js` is what its
//! parity test catches; this one says what an argument must be, and
//! drifting from §16.3.6 is what the tests below catch.

use crate::ty::Constraint;

/// What an element's leading positional argument means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// No leading positional argument at all.
    None,
    /// A value shown as a text node. Optional on `Row` and `Column`,
    /// required on `Text`, `Heading` and `Button`.
    Shown { required: bool },
    /// A two-way binding to a signal of this type (§14B.5). The signal
    /// must be `client`-placed: a keystroke must not silently become a
    /// network write.
    Bound(Bound),
}

/// Which of the two two-way bindings an element uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// `Input` — `value`, bound to a `Text` signal.
    Text,
    /// `Checkbox` — `checked`, bound to a `Truth` signal.
    Truth,
}

/// The argument shape of one built-in element.
#[derive(Debug, Clone, Copy)]
pub struct Signature {
    pub slot: Slot,
    /// A named argument this element requires. `ErrorBar`'s `message` is
    /// the only one: §16.3.6 makes it the element's text.
    pub required_named: Option<&'static str>,
}

/// The signature of `name`, or `None` if it is not a built-in element.
///
/// `zdc-resolve` has already rejected every other name, so `None` here
/// means this table and the resolver's list have drifted.
pub fn signature(name: &str) -> Option<Signature> {
    let signature = match name {
        // §16.3.6 recommends a leading text slot for `Row` and `Column`,
        // because four sources write one and `elements.js` does not have
        // one. It is optional here: `Column` with no argument is the
        // commonest thing in every example.
        "Column" | "Row" => Signature {
            slot: Slot::Shown { required: false },
            required_named: None,
        },
        "Text" | "Heading" | "Button" => Signature {
            slot: Slot::Shown { required: true },
            required_named: None,
        },
        "Input" => Signature {
            slot: Slot::Bound(Bound::Text),
            required_named: None,
        },
        "Checkbox" => Signature {
            slot: Slot::Bound(Bound::Truth),
            required_named: None,
        },
        "Spinner" => Signature {
            slot: Slot::None,
            required_named: None,
        },
        "ErrorBar" => Signature {
            slot: Slot::None,
            required_named: Some("message"),
        },
        _ => return None,
    };
    Some(signature)
}

/// What a named argument must be.
///
/// §16.3.6: `padding is 8` becomes `8px`, `weight` becomes
/// `font-weight`, `hint` becomes `placeholder`, `class` is appended to
/// the base class, and anything else becomes the attribute of that name.
/// An attribute is a string in the DOM, so anything showable will do.
pub fn named_argument(name: &str) -> Constraint {
    match name {
        "padding" => Constraint::Numeric,
        "hint" | "label" | "message" | "weight" | "class" => Constraint::Shown,
        _ => Constraint::Shown,
    }
}

/// Whether a named argument must specifically be `Text` rather than
/// merely showable. Keeping these separate is what makes `hint is 8` an
/// error while `Text 8` is not.
pub fn named_argument_is_text(name: &str) -> bool {
    matches!(name, "hint" | "label" | "message" | "weight" | "class")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_element_the_resolver_accepts_has_a_signature() {
        for name in [
            "Column", "Row", "Text", "Heading", "Button", "Input", "Checkbox", "Spinner",
            "ErrorBar",
        ] {
            assert!(signature(name).is_some(), "{name} has no signature");
        }
    }

    #[test]
    fn the_two_way_elements_are_the_ones_14b5_names() {
        assert!(matches!(
            signature("Input").expect("Input").slot,
            Slot::Bound(Bound::Text)
        ));
        assert!(matches!(
            signature("Checkbox").expect("Checkbox").slot,
            Slot::Bound(Bound::Truth)
        ));
    }

    #[test]
    fn error_bar_takes_its_text_from_a_named_argument() {
        let signature = signature("ErrorBar").expect("ErrorBar");
        assert_eq!(signature.slot, Slot::None);
        assert_eq!(signature.required_named, Some("message"));
    }

    #[test]
    fn padding_is_a_number_and_hint_is_text() {
        assert_eq!(named_argument("padding"), Constraint::Numeric);
        assert!(named_argument_is_text("hint"));
        assert!(!named_argument_is_text("padding"));
    }
}
