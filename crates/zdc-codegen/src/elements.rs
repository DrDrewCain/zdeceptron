//! The built-in element shape table, per spec §16.3.6.
//!
//! The compiler owns the DOM shape of every built-in, which duplicates
//! `elements.js` — and §16.10 names that as a known cost. The mechanism
//! keeping the two honest is the parity test in `tests/element_parity.rs`:
//! for each built-in, with constant arguments, the tree `elements.js`
//! builds must `isEqualNode` the tree this table's markup parses into.

/// What the leading positional argument of an element means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// No leading positional argument. Passing one is a diagnostic.
    None,
    /// One text node, before any children.
    Text,
    /// Two-way: `bindAttr(n, 'value', get)` plus an `input` listener.
    Value,
    /// Two-way: `bindAttr(n, 'checked', get)` plus a `change` listener.
    Checked,
    /// `ErrorBar`, whose text comes from the named `message` argument.
    Message,
}

/// The DOM shape of one built-in element.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub tag: &'static str,
    /// Attributes baked in ahead of anything the program says.
    pub attributes: &'static [(&'static str, &'static str)],
    /// The base class, which a program's own `class` is appended to.
    pub base_class: Option<&'static str>,
    pub slot: Slot,
    /// Whether child nodes may follow.
    pub children: bool,
    /// A literal text child, after the slot: `Spinner`'s ellipsis.
    pub literal_text: Option<&'static str>,
}

/// The shape of `name`, or `None` if it is not a built-in element.
///
/// `zdc-resolve` has already rejected any other name, so `None` here means
/// the two tables have drifted rather than that a program is wrong.
pub fn shape(name: &str) -> Option<Shape> {
    let shape = match name {
        "Column" => Shape {
            tag: "div",
            attributes: &[],
            base_class: Some("zd-col"),
            slot: Slot::None,
            children: true,
            literal_text: None,
        },
        "Row" => Shape {
            tag: "div",
            attributes: &[],
            base_class: Some("zd-row"),
            slot: Slot::None,
            children: true,
            literal_text: None,
        },
        "Text" => Shape {
            tag: "span",
            attributes: &[],
            base_class: None,
            slot: Slot::Text,
            children: false,
            literal_text: None,
        },
        "Heading" => Shape {
            tag: "h2",
            attributes: &[],
            base_class: None,
            slot: Slot::Text,
            children: false,
            literal_text: None,
        },
        "Button" => Shape {
            tag: "button",
            attributes: &[("type", "button")],
            base_class: None,
            slot: Slot::Text,
            children: true,
            literal_text: None,
        },
        "Input" => Shape {
            tag: "input",
            attributes: &[("type", "text")],
            base_class: None,
            slot: Slot::Value,
            children: false,
            literal_text: None,
        },
        "Checkbox" => Shape {
            tag: "input",
            attributes: &[("type", "checkbox")],
            base_class: None,
            slot: Slot::Checked,
            children: false,
            literal_text: None,
        },
        "Spinner" => Shape {
            tag: "span",
            attributes: &[("aria-busy", "true")],
            base_class: None,
            slot: Slot::None,
            children: false,
            literal_text: Some("…"),
        },
        "ErrorBar" => Shape {
            tag: "div",
            attributes: &[("role", "alert")],
            base_class: Some("zd-err"),
            slot: Slot::Message,
            children: false,
            literal_text: None,
        },
        _ => return None,
    };
    Some(shape)
}

/// The class name wrapping a `Checkbox` that was given a `label`.
pub const CHECKBOX_LABEL_CLASS: &str = "zd-row";

/// Every built-in, so a test can iterate the table rather than restate it.
pub const BUILT_INS: &[&str] = &[
    "Column", "Row", "Text", "Heading", "Button", "Input", "Checkbox", "Spinner", "ErrorBar",
];

/// How a named argument reaches the DOM, per `props()` in `elements.js`.
pub enum Named {
    /// A CSS declaration: the property, and whether the value takes `px`.
    Style { property: &'static str, px: bool },
    /// A DOM attribute under a possibly different name.
    Attribute(String),
    /// Appended to the element's base class.
    Class,
    /// Read by the element itself and never written to the DOM.
    Consumed,
}

pub fn named_argument(name: &str) -> Named {
    match name {
        "padding" => Named::Style {
            property: "padding",
            px: true,
        },
        "weight" => Named::Style {
            property: "font-weight",
            px: false,
        },
        "hint" => Named::Attribute("placeholder".to_string()),
        "class" => Named::Class,
        "label" | "message" => Named::Consumed,
        other => Named::Attribute(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_the_resolver_accepts_has_a_shape() {
        for name in BUILT_INS {
            assert!(shape(name).is_some(), "`{name}` has no shape");
        }
    }

    #[test]
    fn an_unknown_element_has_no_shape() {
        assert!(shape("Colunm").is_none());
    }

    #[test]
    fn named_arguments_follow_the_props_mapping() {
        assert!(matches!(
            named_argument("padding"),
            Named::Style { px: true, .. }
        ));
        assert!(matches!(
            named_argument("weight"),
            Named::Style {
                property: "font-weight",
                px: false
            }
        ));
        assert!(matches!(named_argument("hint"), Named::Attribute(name) if name == "placeholder"));
        assert!(matches!(named_argument("message"), Named::Consumed));
        assert!(matches!(named_argument("id"), Named::Attribute(name) if name == "id"));
    }
}
