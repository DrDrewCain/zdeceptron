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
    /// Where a `Link` goes, written first and required. Named apart from
    /// `Shown` because it is a URL, which codegen filters: §16.3.5's
    /// escaping argument covers markup, and a URL is not parsed as markup.
    ///
    /// # One slot, two kinds of value, and why that is not two phrasings
    ///
    /// A destination is either a value of the program's `route` type —
    /// `Link Home`, `Link (BlogPost with slug is post.slug)` — or `Text`,
    /// as in `Link "https://example.com/feed.xml"`.
    ///
    /// §4.1 forbids two phrasings for one construct, and this is one
    /// phrasing: one slot, written first, lowered to one attribute
    /// ([`zdc_hir::DESTINATION_ARGUMENT`]) down one path. What differs is
    /// the *value*, and the two kinds of value name disjoint things. A
    /// route value names a page **this program emits**, and §14G.2
    /// revision 1's whole point is that its URL is rendered by the
    /// compiler from the route table rather than retyped: a mistyped route
    /// is a name that does not resolve and a missing parameter is a
    /// missing field. `Text` names a destination **outside** the program,
    /// which no route can express and which `page.zd` needs.
    ///
    /// The one place the two could overlap is a literal URL that this
    /// program does serve, and `crate::routing` refuses exactly that,
    /// naming the route to write instead. So no destination is expressible
    /// both ways, which is what §4.1 actually asks.
    Destination,
    /// HTML, parsed as HTML. `Prose` and nothing else.
    ///
    /// Not a [`Constraint`] but an exact type: a constraint admits a set,
    /// and the whole point of this slot is that the set has one member.
    /// `Shown` deliberately does not admit `Markup` and this deliberately
    /// does not admit `Text`, so the two slots are disjoint in both
    /// directions and neither element can be given the other's argument.
    Rendered,
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
    /// Named arguments this element requires. `ErrorBar`'s `message` is its
    /// text (§16.3.6); `Image`'s `alt` and `source` are the two an image
    /// has no meaning without.
    pub required_named: &'static [&'static str],
}

/// The signature of `name`, or `None` if it is not a built-in element.
///
/// `zdc-resolve` has already rejected every other name, so `None` here
/// means this table and the resolver's list have drifted.
pub fn signature(name: &str) -> Option<Signature> {
    let slot = match name {
        // §4.4 ratifies a leading text slot for `Row` and `Column`. It is
        // optional: `Column` with no argument is the commonest thing in
        // every example, and `Row item.name` is the row's own text
        // followed by its children.
        "Column" | "Row" => Slot::Shown { required: false },
        // Structure and grouping: everything they show is nested inside.
        "Main" | "Section" | "Article" | "Aside" | "Navigation" | "Header" | "Footer"
        | "Address" | "Divider" | "Break" | "Quote" | "List" | "NumberedList" | "Terms"
        | "Figure" | "Canvas" | "Fieldset" | "Spinner" => Slot::None,
        // The text they show is the whole element.
        "Text" | "Heading" | "Button" | "Emphasis" | "Strong" | "Code" | "Key" | "Time"
        | "Term" | "Small" | "Mark" | "Abbreviation" | "Superscript" | "Subscript" | "Label"
        | "Legend" => Slot::Shown { required: true },
        // Text, or children, or both.
        "Paragraph" | "CodeBlock" | "Preformatted" | "Item" | "Description" | "Caption" => {
            Slot::Shown { required: false }
        }
        "Link" => Slot::Destination,
        "Prose" => Slot::Rendered,
        "Image" => Slot::None,
        "Input" => Slot::Bound(Bound::Text),
        "Checkbox" => Slot::Bound(Bound::Truth),
        "ErrorBar" => Slot::None,
        _ => return None,
    };
    let required_named: &'static [&'static str] = match name {
        "ErrorBar" => &["message"],
        "Image" => &["source", "alt"],
        "Abbreviation" => &["expansion"],
        "Label" => &["controls"],
        _ => &[],
    };
    Some(Signature {
        slot,
        required_named,
    })
}

/// What a named argument must be.
///
/// §16.3.6: `padding is 8` becomes `8px`, `weight` becomes
/// `font-weight`, `hint` becomes `placeholder`, `class` is appended to
/// the base class. Codegen has already refused any name outside the
/// element's own set, so this table need only cover the permitted ones —
/// an attribute is a string in the DOM, so anything showable will do.
pub fn named_argument(name: &str) -> Constraint {
    match name {
        "padding" | "width" | "height" => Constraint::Numeric,
        _ => Constraint::Shown,
    }
}

/// Whether a named argument must specifically be `Text` rather than
/// merely showable. Keeping these separate is what makes `hint is 8` an
/// error while `Text 8` is not.
pub fn named_argument_is_text(name: &str) -> bool {
    matches!(
        name,
        "hint"
            | "label"
            | "message"
            | "weight"
            | "class"
            | "source"
            | "alt"
            | "controls"
            | "exact"
            | "expansion"
            | "rel"
            | "loading"
            | "id"
            | "title"
            | "role"
            | "lang"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resolver's list is the authority on what a program may write,
    /// so this reads it rather than restating it: a name added there and
    /// forgotten here would otherwise typecheck as an unknown element.
    #[test]
    fn every_element_the_resolver_accepts_has_a_signature() {
        // Counted: the assertion is inside the loop, so an emptied
        // vocabulary would pass this over nothing.
        let mut scanned = 0;
        for name in zdc_resolve::BUILTIN_ELEMENTS {
            scanned += 1;
            assert!(signature(name).is_some(), "{name} has no signature");
        }
        assert_eq!(
            scanned,
            zdc_resolve::BUILTIN_ELEMENTS.len(),
            "every element in the vocabulary must be checked"
        );
        assert!(scanned >= 36, "the element vocabulary shrank: {scanned}");
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
    fn an_image_must_say_what_it_is_and_where_it_is() {
        let signature = signature("Image").expect("Image");
        assert_eq!(signature.required_named, ["source", "alt"]);
    }

    #[test]
    fn a_link_leads_with_where_it_goes() {
        assert_eq!(signature("Link").expect("Link").slot, Slot::Destination);
    }

    #[test]
    fn padding_is_a_number_and_hint_is_text() {
        assert_eq!(named_argument("padding"), Constraint::Numeric);
        assert!(named_argument_is_text("hint"));
        assert!(!named_argument_is_text("padding"));
    }
}
