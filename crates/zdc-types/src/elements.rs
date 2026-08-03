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
    /// Named arguments this element requires. `ErrorBar`'s `message` is
    /// its text (§16.3.6); `Image`'s `source` and `alt`, and `Link`'s
    /// `href`, are the ones the element has no meaning without.
    pub required_named: &'static [&'static str],
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
            required_named: &[],
        },
        "Text" | "Heading" | "Button" => Signature {
            slot: Slot::Shown { required: true },
            required_named: &[],
        },
        "Input" => Signature {
            slot: Slot::Bound(Bound::Text),
            required_named: &[],
        },
        "Checkbox" => Signature {
            slot: Slot::Bound(Bound::Truth),
            required_named: &[],
        },
        "Spinner" => Signature {
            slot: Slot::None,
            required_named: &[],
        },
        "ErrorBar" => Signature {
            slot: Slot::None,
            required_named: &["message"],
        },
        // An image is two named arguments and nothing else: where it comes
        // from, and what it says to a reader who cannot see it. `source`
        // is a URL, which is why §16.3.5's escaping argument does not
        // reach it and `zdc-hir::is_url_attribute` does.
        "Image" => Signature {
            slot: Slot::None,
            required_named: &["source", "alt"],
        },
        // A link's destination is `href is …`, named rather than leading,
        // so that every URL in the language arrives through one door: the
        // named-argument list the sink rule ranges over.
        //
        // **For whoever merges this with a `Link` that takes its
        // destination positionally.** A leading argument is lowered by the
        // slot, not by `named_argument`, and the slot never reaches
        // `zdc_hir::is_url_attribute` — so a positional `Link "/notes"`
        // would be a URL the sink rule never sees, and sink 7 would be
        // silently undone for the commonest way of writing a link. The
        // positional slot must be routed through `URL_ATTRIBUTES` as though
        // it were spelled `href`, or the two forms must not both exist.
        "Link" => Signature {
            slot: Slot::None,
            required_named: &["href"],
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
    matches!(
        name,
        "hint" | "label" | "message" | "weight" | "class" | "source" | "href" | "alt" | "rel"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_element_the_resolver_accepts_has_a_signature() {
        for element in zdc_hir::BuiltinElement::ALL {
            assert!(
                signature(element.name()).is_some(),
                "{} has no signature",
                element.name()
            );
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
        assert_eq!(signature.required_named, ["message"]);
    }

    #[test]
    fn padding_is_a_number_and_hint_is_text() {
        assert_eq!(named_argument("padding"), Constraint::Numeric);
        assert!(named_argument_is_text("hint"));
        assert!(!named_argument_is_text("padding"));
    }
}
