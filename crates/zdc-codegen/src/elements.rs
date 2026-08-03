//! The built-in element shape table, per spec §16.3.6.
//!
//! The compiler owns the DOM shape of every built-in, which duplicates
//! `elements.js` — and §16.10 names that as a known cost. The mechanism
//! keeping the two honest is the parity test in `tests/element_parity.rs`:
//! for each built-in, with constant arguments, the tree `elements.js`
//! builds must `isEqualNode` the tree this table's markup parses into.
//!
//! # Why the vocabulary is a table and not an escape hatch
//!
//! §16.1 chose template cloning, which is sound only while the tag name is
//! a compile-time constant. A construct naming a raw tag — `Element "p"` —
//! would keep that property for a string literal, but it would also give
//! the language two ways to write a paragraph the moment any semantic name
//! exists, and §4.1 forbids exactly that. It would additionally reopen the
//! attribute set, which is the injection surface `attributes` below closes.
//!
//! So the vocabulary widens the only way it can widen without giving
//! either of those up: one name per element, chosen for what the element
//! *means* rather than for the tag it becomes, and every tag still a
//! `&'static str` in this file.

/// What the leading positional argument of an element means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// No leading positional argument. Passing one is a diagnostic.
    None,
    /// One text node, before any children, and it must be written.
    Text,
    /// One text node, before any children, which may be omitted — the
    /// element's content can come from its children instead.
    OptionalText,
    /// A URL, which becomes `href`. `Link`'s content is its children, so
    /// the leading argument is where the link *goes*, matching §14G.2's
    /// `Link Home` — the destination first, the content nested under it.
    ///
    /// It holds either a value of the program's `route` type, whose URL
    /// the compiler renders from the route table (§14G.2 revision 1), or
    /// a `Text` naming somewhere outside the program. Both leave here as
    /// a URL and take the same filtered path to `href`, which is what
    /// keeps a destination from being a URL no rule over URL-bearing
    /// attributes ever sees.
    Destination,
    /// Two-way: `bindAttr(n, 'value', get)` plus an `input` listener.
    Value,
    /// Two-way: `bindAttr(n, 'checked', get)` plus a `change` listener.
    Checked,
    /// `ErrorBar`, whose text comes from the named `message` argument.
    Message,
    /// HTML, parsed as HTML and made this element's whole content.
    ///
    /// The only slot in the table whose value is not escaped, because it
    /// is the only one whose type guarantees it does not need to be
    /// (`zdc_types::Type::Markup`).
    Rendered,
}

/// The DOM shape of one built-in element.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    /// The tag, except for a heading — see `heading`.
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
    /// Whether `tag` is chosen by nesting depth rather than fixed.
    ///
    /// A heading's level is not something the program writes, so it cannot
    /// be written wrongly: `Heading` at the top of the document is `h1`,
    /// and one level deeper for each enclosing `Section` or `Aside`. An
    /// outline that skips a level, or starts at `h3`, is not expressible.
    pub heading: bool,
    /// Whether a heading inside this element is a level deeper.
    ///
    /// `Section` and `Aside` and nothing else: they are the two elements
    /// whose content is subordinate to what surrounds it. `Article`,
    /// `Main` and `Navigation` are landmarks carrying a page's own
    /// content, and deepening inside them would leave the commonest page
    /// there is — one article, one heading — with no `h1` at all, which is
    /// itself a standard accessibility failure.
    pub sectioning: bool,
    /// Named arguments this element accepts beyond `GLOBAL_ARGUMENTS`.
    pub arguments: &'static [&'static str],
    /// Named arguments that must be given.
    pub required_arguments: &'static [&'static str],
    /// The elements this one's direct element children must be, or `&[]`
    /// when any child will do.
    pub only_children: &'static [&'static str],
    /// The elements this one may be written directly inside, or `&[]` when
    /// it may be written anywhere.
    pub only_inside: &'static [&'static str],
}

/// The shape every entry below starts from, so a row states only what is
/// unusual about it.
const PLAIN: Shape = Shape {
    tag: "div",
    attributes: &[],
    base_class: None,
    slot: Slot::None,
    children: true,
    literal_text: None,
    heading: false,
    sectioning: false,
    arguments: &[],
    required_arguments: &[],
    only_children: &[],
    only_inside: &[],
};

/// The heading tags, indexed by nesting depth and clamped at the last.
pub const HEADING_TAGS: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6"];

/// The tag a heading takes at `depth` sectioning elements deep.
pub fn heading_tag(depth: usize) -> &'static str {
    HEADING_TAGS[depth.min(HEADING_TAGS.len() - 1)]
}

/// The shape of `name`, or `None` if it is not a built-in element.
///
/// `zdc-resolve` has already rejected any other name, so `None` here means
/// the two tables have drifted rather than that a program is wrong.
pub fn shape(name: &str) -> Option<Shape> {
    let shape = match name {
        // --- layout -------------------------------------------------------
        //
        // Both take an optional leading text slot, ratified in §4.4 and
        // recorded in §16.3.6's table. `Row item.name` is one text node
        // followed by the row's children, which is a different tree from
        // `Row` with a nested `Text item.name` — the first has a bare text
        // node, the second a `<span>` — so §4.1 is satisfied: the two
        // phrasings say different things rather than the same thing twice.
        "Column" => Shape {
            base_class: Some("zd-col"),
            slot: Slot::OptionalText,
            ..PLAIN
        },
        "Row" => Shape {
            base_class: Some("zd-row"),
            slot: Slot::OptionalText,
            ..PLAIN
        },

        // --- document structure -------------------------------------------
        "Main" => Shape {
            tag: "main",
            ..PLAIN
        },
        "Section" => Shape {
            tag: "section",
            sectioning: true,
            ..PLAIN
        },
        "Article" => Shape {
            tag: "article",
            ..PLAIN
        },
        "Aside" => Shape {
            tag: "aside",
            sectioning: true,
            ..PLAIN
        },
        "Navigation" => Shape {
            tag: "nav",
            ..PLAIN
        },
        "Header" => Shape {
            tag: "header",
            ..PLAIN
        },
        "Footer" => Shape {
            tag: "footer",
            ..PLAIN
        },
        "Divider" => Shape {
            tag: "hr",
            children: false,
            ..PLAIN
        },

        // --- text ---------------------------------------------------------
        "Text" => Shape {
            tag: "span",
            slot: Slot::Text,
            children: false,
            ..PLAIN
        },
        "Heading" => Shape {
            tag: "h1",
            slot: Slot::Text,
            children: false,
            heading: true,
            ..PLAIN
        },
        "Paragraph" => Shape {
            tag: "p",
            slot: Slot::OptionalText,
            ..PLAIN
        },
        "Emphasis" => Shape {
            tag: "em",
            slot: Slot::Text,
            children: false,
            ..PLAIN
        },
        "Strong" => Shape {
            tag: "strong",
            slot: Slot::Text,
            children: false,
            ..PLAIN
        },
        "Code" => Shape {
            tag: "code",
            slot: Slot::Text,
            children: false,
            ..PLAIN
        },
        "CodeBlock" => Shape {
            tag: "pre",
            slot: Slot::OptionalText,
            ..PLAIN
        },
        "Quote" => Shape {
            tag: "blockquote",
            ..PLAIN
        },
        "Key" => Shape {
            tag: "kbd",
            slot: Slot::Text,
            children: false,
            ..PLAIN
        },
        "Time" => Shape {
            tag: "time",
            slot: Slot::Text,
            children: false,
            arguments: &["exact"],
            ..PLAIN
        },

        // --- rendered documents -------------------------------------------
        //
        // The one element whose content is *parsed* rather than assigned as
        // a text node, and therefore the one place in the whole emitter
        // where a runtime value reaches an HTML parser. §16.3.5's property
        // — no runtime value is ever parsed as markup — is narrowed here
        // rather than abandoned: what reaches the parser is a `Markup`, and
        // `zdc-codegen::capability::markdown` is the only thing in the
        // language that makes one.
        //
        // No children: a document's content is the document. Allowing
        // element children would mean interleaving parsed HTML with cloned
        // template nodes, and the sibling offsets every binding is scheduled
        // against would then depend on how many nodes the *file* parsed
        // into, which is not known at compile time.
        "Prose" => Shape {
            tag: "div",
            base_class: Some("zd-prose"),
            slot: Slot::Rendered,
            children: false,
            ..PLAIN
        },

        // --- lists --------------------------------------------------------
        "List" => Shape {
            tag: "ul",
            only_children: &["Item"],
            ..PLAIN
        },
        "NumberedList" => Shape {
            tag: "ol",
            only_children: &["Item"],
            ..PLAIN
        },
        "Item" => Shape {
            tag: "li",
            slot: Slot::OptionalText,
            only_inside: &["List", "NumberedList"],
            ..PLAIN
        },
        "Terms" => Shape {
            tag: "dl",
            only_children: &["Term", "Description"],
            ..PLAIN
        },
        "Term" => Shape {
            tag: "dt",
            slot: Slot::Text,
            children: false,
            only_inside: &["Terms"],
            ..PLAIN
        },
        "Description" => Shape {
            tag: "dd",
            slot: Slot::OptionalText,
            only_inside: &["Terms"],
            ..PLAIN
        },

        // --- links and media ----------------------------------------------
        "Link" => Shape {
            tag: "a",
            slot: Slot::Destination,
            arguments: &["rel"],
            ..PLAIN
        },
        "Image" => Shape {
            tag: "img",
            children: false,
            // `alt` is required, not defaulted. An image with no text
            // alternative is the single commonest accessibility failure,
            // and a default would silently produce one.
            arguments: &["source", "alt", "width", "height", "loading"],
            required_arguments: &["source", "alt"],
            ..PLAIN
        },
        "Figure" => Shape {
            tag: "figure",
            ..PLAIN
        },
        "Caption" => Shape {
            tag: "figcaption",
            slot: Slot::OptionalText,
            only_inside: &["Figure"],
            ..PLAIN
        },
        "Canvas" => Shape {
            tag: "canvas",
            children: false,
            arguments: &["width", "height"],
            ..PLAIN
        },

        // --- controls -----------------------------------------------------
        "Button" => Shape {
            tag: "button",
            attributes: &[("type", "button")],
            slot: Slot::Text,
            ..PLAIN
        },
        "Input" => Shape {
            tag: "input",
            attributes: &[("type", "text")],
            slot: Slot::Value,
            children: false,
            arguments: &["hint"],
            ..PLAIN
        },
        "Checkbox" => Shape {
            tag: "input",
            attributes: &[("type", "checkbox")],
            slot: Slot::Checked,
            children: false,
            arguments: &["label"],
            ..PLAIN
        },
        "Spinner" => Shape {
            tag: "span",
            attributes: &[("aria-busy", "true")],
            children: false,
            literal_text: Some("…"),
            ..PLAIN
        },
        "ErrorBar" => Shape {
            tag: "div",
            attributes: &[("role", "alert")],
            base_class: Some("zd-err"),
            slot: Slot::Message,
            children: false,
            arguments: &["message"],
            required_arguments: &["message"],
            ..PLAIN
        },
        _ => return None,
    };
    Some(shape)
}

/// The class name wrapping a `Checkbox` that was given a `label`.
pub const CHECKBOX_LABEL_CLASS: &str = "zd-row";

/// Every built-in, so a test can iterate the table rather than restate it.
///
/// One name per element, and no name is a synonym for another (§4.1): the
/// list is the whole vocabulary and there is no second way to reach a tag
/// on it.
pub const BUILT_INS: &[&str] = zdc_hir::BuiltinElement::NAMES;

/// The named arguments every element accepts.
///
/// The set is **closed**, here and per element, and a name outside it is a
/// diagnostic rather than an attribute. An open set makes every element an
/// injection surface: `onclick`, `srcdoc`, `formaction` and `style` are all
/// ordinary attribute names, and `setAttribute` applies them faithfully.
/// Nothing in §16.3.5's escaping argument covers them, because that
/// argument is about markup parsing and these are not parsed.
///
/// `aria-*` and `data-*` are **not** open, and cannot be: a ZD identifier
/// is UAX#31, which admits no hyphen, so neither is even spellable as an
/// argument name. Where an ARIA attribute carries meaning the language
/// already models it — `role` is here, `Spinner` bakes in `aria-busy`,
/// `ErrorBar` bakes in `role="alert"`, and `Image` requires `alt`.
pub const GLOBAL_ARGUMENTS: &[&str] = &[
    "class", "padding", "weight", "id", "title", "role", "lang", "hidden",
];

/// Whether `element` accepts the named argument `name`.
pub fn accepts_argument(shape: &Shape, name: &str) -> bool {
    GLOBAL_ARGUMENTS.contains(&name) || shape.arguments.contains(&name)
}

/// How a named argument reaches the DOM, per `props()` in `elements.js`.
pub enum Named {
    /// A CSS declaration: the property, and whether the value takes `px`.
    Style { property: &'static str, px: bool },
    /// A DOM attribute under a possibly different name.
    Attribute(&'static str),
    /// A DOM attribute holding a URL, which is filtered before it is set.
    Url(&'static str),
    /// Appended to the element's base class.
    Class,
    /// Read by the element itself and never written to the DOM.
    Consumed,
}

/// The DOM meaning of a permitted named argument.
///
/// Total over `GLOBAL_ARGUMENTS` and every `Shape::arguments` entry, which
/// `named_arguments_are_total` below checks; `accepts_argument` has already
/// rejected everything else.
pub fn named_argument(name: &str) -> Option<Named> {
    let named = match name {
        "padding" => Named::Style {
            property: "padding",
            px: true,
        },
        "weight" => Named::Style {
            property: "font-weight",
            px: false,
        },
        "class" => Named::Class,
        "hint" => Named::Attribute("placeholder"),
        "exact" => Named::Attribute("datetime"),
        "source" => Named::Url("src"),
        "id" => Named::Attribute("id"),
        "title" => Named::Attribute("title"),
        "role" => Named::Attribute("role"),
        "lang" => Named::Attribute("lang"),
        "hidden" => Named::Attribute("hidden"),
        "alt" => Named::Attribute("alt"),
        "width" => Named::Attribute("width"),
        "height" => Named::Attribute("height"),
        "loading" => Named::Attribute("loading"),
        "rel" => Named::Attribute("rel"),
        "label" | "message" => Named::Consumed,
        _ => return None,
    };
    Some(named)
}

/// The URL schemes a `Link` or an `Image` may name.
///
/// Everything else is refused. `javascript:` is script execution behind a
/// click; `data:` is a same-origin document an attacker fully controls;
/// `vbscript:` is both. A URL with no scheme at all — `/work`, `./a.png`,
/// `#top` — is relative and always allowed.
///
/// The list itself lives in `zdc_hir`, because the information-flow pass
/// and the markdown renderer rule on the same URLs and
/// `crates/zdc-codegen/tests/url.rs` runs that one list against `safeUrl`
/// in a real JavaScript engine. A second copy here would be a copy the
/// JavaScript half is never compared against, and none of the three
/// executing schemes should depend on which of two lists a reader
/// happened to open.
pub use zdc_hir::URL_SCHEMES;

/// Whether a compile-time-known URL may be emitted.
///
/// One line, delegating: this rule is `zdc_hir::url_is_safe`, and the
/// emitter is one of its callers rather than a second author of it.
pub fn url_is_permitted(url: &str) -> bool {
    zdc_hir::url_is_safe(url)
}

/// Characters a folded style value may not contain, and what each of them
/// would do to the generated stylesheet.
///
/// `Styles::stylesheet` *prints* `{property}: {value};` into a rule, so
/// unlike `bindStyle` — which hands one declaration to `setProperty` and
/// has the CSSOM drop anything after it — a value here is not confined to
/// its declaration. `weight is "bold; } body { display: none } x {"` is a
/// rule for `body`, which is a defacement of the whole page; `url(…)` in
/// one is an outbound request the program never wrote.
///
/// A line break is on the list since block text literals landed. It ends
/// no rule — CSS reads it as whitespace — but a value is now able to
/// carry one, and a declaration printed across four lines of a generated
/// stylesheet is not a style anybody wrote on purpose. This set refuses
/// rather than escapes, so the ruling for a newly reachable character is
/// the same as for every other one.
pub const STYLE_VALUE_FORBIDDEN: &[char] = &[
    ';', '{', '}', '<', '>', '\\', '"', '\'', '(', ')', '@', ':', '\n', '\r',
];

/// The same set, spelled for a diagnostic.
pub const STYLE_VALUE_FORBIDDEN_NAMES: &[&str] = &[
    ";",
    "{",
    "}",
    "<",
    ">",
    "\\",
    "\"",
    "'",
    "(",
    ")",
    "@",
    ":",
    "/*",
    "a line break",
];

/// Whether a style value may be folded into the generated stylesheet.
pub fn style_value_is_permitted(value: &str) -> bool {
    !value.contains(STYLE_VALUE_FORBIDDEN) && !value.contains("/*")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_style_value_may_not_end_the_rule_it_is_folded_into() {
        assert!(style_value_is_permitted("bold"));
        assert!(style_value_is_permitted("600"));
        assert!(style_value_is_permitted("8px"));
        assert!(style_value_is_permitted("lighter"));
        assert!(!style_value_is_permitted(
            "bold; } body { display: none } x {"
        ));
        assert!(!style_value_is_permitted("bold;"));
        assert!(!style_value_is_permitted("normal /* x */"));
        assert!(!style_value_is_permitted("url(https://example.com/x)"));
        assert!(
            !style_value_is_permitted("bold\nnormal"),
            "a block text literal can carry a line break into a style value"
        );
    }

    #[test]
    fn every_built_in_the_resolver_accepts_has_a_shape() {
        let mut checked = 0;
        for name in BUILT_INS {
            assert!(shape(name).is_some(), "`{name}` has no shape");
            checked += 1;
        }
        // An empty vocabulary would satisfy the loop over nothing.
        assert_eq!(
            checked,
            zdc_hir::BuiltinElement::ALL.len(),
            "the shape table was checked against {checked} names"
        );
    }

    #[test]
    fn the_shape_table_covers_the_whole_vocabulary() {
        for element in zdc_hir::BuiltinElement::ALL {
            assert!(
                shape(element.name()).is_some(),
                "`{}` has no shape",
                element.name()
            );
            assert!(
                BUILT_INS.contains(&element.name()),
                "`{}` is missing from BUILT_INS",
                element.name()
            );
        }
        assert_eq!(BUILT_INS.len(), zdc_hir::BuiltinElement::ALL.len());
    }

    /// `source` is the ZDeceptron spelling and `src` is what reaches the
    /// DOM — and it arrives as a *filtered* attribute, not a plain one:
    /// an image source is a request the browser issues to whatever host
    /// the value names (§16.3.5, corrected).
    #[test]
    fn the_image_source_reaches_the_dom_as_a_filtered_src() {
        assert!(matches!(named_argument("source"), Some(Named::Url("src"))));
    }

    #[test]
    fn an_unknown_element_has_no_shape() {
        assert!(shape("Colunm").is_none());
        assert!(shape("Element").is_none());
        assert!(shape("Script").is_none());
    }

    #[test]
    fn named_arguments_are_total_over_the_permitted_set() {
        let mut scanned = 0;
        for name in GLOBAL_ARGUMENTS {
            scanned += 1;
            assert!(
                named_argument(name).is_some(),
                "`{name}` is accepted everywhere but has no DOM meaning"
            );
        }
        for element in BUILT_INS {
            let shape = shape(element).expect("a built-in has a shape");
            for name in shape.arguments {
                scanned += 1;
                assert!(
                    named_argument(name).is_some(),
                    "`{element}` accepts `{name}`, which has no DOM meaning"
                );
            }
            for name in shape.required_arguments {
                scanned += 1;
                assert!(
                    shape.arguments.contains(name),
                    "`{element}` requires `{name}` but does not accept it"
                );
            }
        }
        // Emptying either table would satisfy every loop above over
        // nothing at all, which is the shape this counts against. The
        // floor is a literal, not a length derived from the same tables
        // the loops walk: emptying those would move both numbers to zero
        // and the assertion would agree with itself. Bumping it when an
        // argument is added is the point, not the cost.
        assert!(
            scanned >= 20,
            "the argument tables shrank: only {scanned} names were checked"
        );
        assert!(
            BUILT_INS.len() >= 36,
            "the element vocabulary shrank to {}",
            BUILT_INS.len()
        );
    }

    #[test]
    fn named_arguments_follow_the_props_mapping() {
        assert!(matches!(
            named_argument("padding"),
            Some(Named::Style { px: true, .. })
        ));
        assert!(matches!(
            named_argument("weight"),
            Some(Named::Style {
                property: "font-weight",
                px: false
            })
        ));
        assert!(matches!(
            named_argument("hint"),
            Some(Named::Attribute("placeholder"))
        ));
        assert!(matches!(named_argument("message"), Some(Named::Consumed)));
        assert!(matches!(named_argument("id"), Some(Named::Attribute("id"))));
    }

    #[test]
    fn the_attribute_set_is_closed() {
        let column = shape("Column").expect("Column");
        assert!(accepts_argument(&column, "id"));
        assert!(!accepts_argument(&column, "onclick"));
        assert!(!accepts_argument(&column, "style"));
        assert!(!accepts_argument(&column, "srcdoc"));
        // `hint` belongs to `Input` and to nothing else.
        assert!(!accepts_argument(&column, "hint"));
        assert!(accepts_argument(&shape("Input").expect("Input"), "hint"));
    }

    #[test]
    fn a_heading_level_is_its_depth() {
        assert_eq!(heading_tag(0), "h1");
        assert_eq!(heading_tag(1), "h2");
        assert_eq!(heading_tag(5), "h6");
        // Deeper than six clamps, exactly as HTML's own outline did.
        assert_eq!(heading_tag(9), "h6");
    }

    #[test]
    fn script_bearing_urls_are_refused() {
        assert!(url_is_permitted("/work"));
        assert!(url_is_permitted("#top"));
        assert!(url_is_permitted("https://example.com/a:b"));
        assert!(url_is_permitted("mailto:a@example.com"));
        assert!(!url_is_permitted("javascript:alert(1)"));
        assert!(!url_is_permitted("JavaScript:alert(1)"));
        assert!(!url_is_permitted("  javascript:alert(1)"));
        assert!(!url_is_permitted("data:text/html,<script>"));
    }
}
