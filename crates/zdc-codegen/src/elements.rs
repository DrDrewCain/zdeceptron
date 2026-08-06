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

use crate::style::{Condition, Grammar};

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
    /// A number, bound one way into the `value` attribute.
    ///
    /// One way, and that is what tells it apart from [`Slot::Value`]: a
    /// `progress` and a `meter` are read and not written, so there is no
    /// listener and no §14B.5 placement rule to apply. It is also not
    /// [`Slot::Text`], because the browser reads the number and draws it
    /// rather than showing it, and a value that is not a number leaves
    /// the element at zero with no diagnostic anywhere.
    Amount,
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
    /// The element this one's first element child must be, or `None`.
    ///
    /// Two elements set it, and both for the same reason: `Fieldset` and
    /// `Details` each have a child that supplies the accessible name of
    /// the whole thing, and each renders *worse* without it than the plain
    /// markup would. A `fieldset` with no `legend` is announced as an
    /// unnamed group before every control inside it; a `details` with no
    /// `summary` gets whatever word the browser chose, in whatever
    /// language the browser chose it in. Refusing follows `Image`'s `alt`:
    /// the element that needs a name asks for one rather than inventing
    /// it.
    pub leading_child: Option<&'static str>,
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
    leading_child: None,
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
        // How to reach the people behind the nearest `Article`, or behind
        // the page. Not a postal address, despite the name HTML gave it:
        // an email address, a phone number and a link to a profile are all
        // the same claim, and it is the claim assistive technology and a
        // crawler both read as "this is who wrote it".
        //
        // Everything it shows is nested inside, so there is no leading
        // slot: a contact block is a `Link`, a `Text` and often a `Break`,
        // rather than one run of text.
        "Address" => Shape {
            tag: "address",
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
        // Preserved whitespace that is not code: a poem, a signature
        // block, an ASCII drawing, a transcript.
        //
        // It is a `pre`, and so is `CodeBlock`, which would make two
        // spellings of one thing if the two rendered alike. They do not.
        // `zd-pre` sets the document's own typeface and lets long lines
        // wrap, because a poem in a monospace font that scrolls sideways
        // is a poem rendered as code; `CodeBlock` keeps the browser's
        // monospace default and its refusal to wrap, because a wrapped
        // line of code is a line of code that has been lied about. Two
        // elements, two renderings, two claims about what the text is.
        "Preformatted" => Shape {
            tag: "pre",
            base_class: Some("zd-pre"),
            slot: Slot::OptionalText,
            ..PLAIN
        },
        // A line break inside a run of text: the second line of an address,
        // the next line of a verse. Not a paragraph separator, which is
        // what `Paragraph` is for, and a `Break` between two blocks is
        // vertical space nobody asked for.
        //
        // No slot and no children, because it is not a container: it is
        // the boundary between two things written beside it.
        "Break" => Shape {
            tag: "br",
            children: false,
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
        // Fine print, and it is a semantic rather than a size. `small`
        // means "this is an aside the reader may skip", which is what a
        // disclaimer, a copyright line or a licence note is; the browser's
        // smaller rendering follows from that rather than being the point.
        // `size is "small"` remains the way to say "smaller", and the two
        // do not overlap: one is a claim about the text and the other is a
        // claim about its measurements.
        "Small" => Shape {
            tag: "small",
            slot: Slot::Text,
            children: false,
            ..PLAIN
        },
        // Highlighted because it is relevant *here*, which is what a
        // search result, a diff and a filtered list are saying. `mark`
        // carries that meaning; a background colour carries only the
        // colour, and a reader who cannot see colour gets nothing from it.
        "Mark" => Shape {
            tag: "mark",
            slot: Slot::Text,
            children: false,
            ..PLAIN
        },
        // The expansion is required, for the reason `Image`'s `alt` is: an
        // `abbr` with no `title` is an acronym with nothing behind it, so
        // the element would be pure decoration and the reader who needed
        // it would be the one who did not get it.
        //
        // Spelled `expansion` rather than `title` even though `title` is
        // the attribute and is already a global argument. What the writer
        // knows is what the letters stand for; that this reaches the DOM
        // as `title` is the table's business, and the table is where CSS's
        // and HTML's vocabularies are translated everywhere else here.
        "Abbreviation" => Shape {
            tag: "abbr",
            slot: Slot::Text,
            children: false,
            arguments: &["expansion"],
            required_arguments: &["expansion"],
            ..PLAIN
        },
        // Raised and lowered text. Two elements rather than one with a
        // direction argument, because a direction argument would be a
        // closed set of two words and the two words are already element
        // names in every markup language there has ever been.
        //
        // They carry meaning rather than position: a screen reader
        // announces `sub` in a formula and `sup` in an ordinal
        // differently from surrounding text, which is the whole reason
        // not to write these as a smaller font raised by a margin.
        "Superscript" => Shape {
            tag: "sup",
            slot: Slot::Text,
            children: false,
            ..PLAIN
        },
        "Subscript" => Shape {
            tag: "sub",
            slot: Slot::Text,
            children: false,
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
        // A video, with controls that cannot be turned off.
        //
        // There is no `controls` argument, and that is the decision rather
        // than an omission. A media element without controls can be
        // started and stopped by a pointer and by nothing else: no
        // keyboard, no screen reader, no way to pause a thing that is
        // moving. The uses for turning them off are a background loop and
        // a player built out of `Button`s, and neither is expressible
        // here anyway, because nothing in the language can start or stop
        // playback.
        //
        // What is *not* claimed: this element carries no captions. A
        // caption is a `track`, which is a child element with a URL and a
        // language of its own, and inventing an empty one would produce a
        // video that says it is captioned and is not. Until that lands, a
        // video here is a video with no text alternative, and the honest
        // place to say so is here.
        //
        // `width` and `height` are attributes, as they are on `Image`,
        // for the same reason: they reserve the layout box before the file
        // arrives, and no stylesheet rule can do that.
        "Video" => Shape {
            tag: "video",
            attributes: &[("controls", "")],
            children: false,
            arguments: &["source", "poster", "width", "height"],
            required_arguments: &["source"],
            ..PLAIN
        },
        // Audio, on the same terms as `Video` and for the same reasons:
        // one filtered URL, controls that cannot be turned off, and no
        // captions claimed. No `poster` and no measurements, because an
        // audio element has no picture and its box is the controls the
        // browser draws.
        "Audio" => Shape {
            tag: "audio",
            attributes: &[("controls", "")],
            children: false,
            arguments: &["source"],
            required_arguments: &["source"],
            ..PLAIN
        },
        // An embedded document, and the one element in the vocabulary that
        // is a trust boundary rather than a shape.
        //
        // # What an embed may reach, decided here
        //
        // Everything below is baked and none of it is an argument, which
        // is the decision. An embed loads a document this compiler cannot
        // see, from a host it does not control, into the reader's browser,
        // and the platform's default is that the document gets a great
        // deal: script execution, form submission, top-level navigation of
        // the embedding page, popups, and — with `allow-same-origin` — the
        // embedder's own origin, which is its cookies and its storage.
        //
        // `sandbox=""` is the empty token list, which grants none of them.
        // A frame here runs no script, submits no form, navigates nothing
        // but itself, opens no window, and has an opaque origin, so it can
        // read nothing of the page that embedded it. `referrerpolicy` is
        // `no-referrer` because the embedded host is otherwise told which
        // page embedded it, on every request, which is a fact about the
        // reader rather than about the document. `loading="lazy"` because
        // an embed below the fold is a request to a third party the reader
        // may never scroll to.
        //
        // **The sandbox is not widenable, and that is the argued part.**
        // The obvious alternative is an `allows` argument taking a closed
        // set of tokens, and it is rejected: every token in that attribute
        // is a capability granted to code the compiler never reads, and
        // `allow-scripts allow-same-origin` together are exactly equivalent
        // to no sandbox at all, because the framed script can then reach
        // into the embedder and remove the attribute. That is a pair a
        // program could write by accident and no diagnostic here could
        // honestly rule on. A program that needs a scripted third party
        // writes a `Link` to it, which is a navigation the reader chooses.
        //
        // `title` is required, because an `iframe` with none is announced
        // as "frame" and nothing else, which is the same failure `Image`
        // requires `alt` to prevent.
        "Frame" => Shape {
            tag: "iframe",
            attributes: &[
                ("sandbox", ""),
                ("referrerpolicy", "no-referrer"),
                ("loading", "lazy"),
            ],
            children: false,
            arguments: &["source", "title", "width", "height"],
            required_arguments: &["source", "title"],
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
        // A submit boundary with one handler.
        //
        // What it buys is what the browser does and a `Column` full of
        // inputs cannot: Enter inside any field fires one `submit`, the
        // event happens after every field has written its value, and
        // assistive technology is told the controls belong together. None
        // of that can be hand-rolled from key handlers without getting the
        // per-control rules wrong.
        //
        // # `on submit` is required
        //
        // A `form` with no submit handler navigates: the browser reloads
        // the current URL with the fields as a query string, and every
        // client signal on the page is gone. That is a worse page than the
        // `Column` this replaces, and it fails silently, at the one moment
        // somebody presses Enter. So the handler is required, and the
        // emitter calls `preventDefault` on the event before running it,
        // which is the other half: a handler that ran and then navigated
        // anyway would be the same loss one frame later.
        //
        // # There is no `action`
        //
        // `action` is a URL-bearing attribute and submission here is a
        // handler this program runs, so a form never navigates to a URL it
        // names. That is why `Form` carries no URL argument at all.
        "Form" => Shape {
            tag: "form",
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
        // A paragraph a person writes, bound exactly as `Input` is.
        //
        // A `textarea` and not an `input` with a taller box: the two differ
        // in what the Enter key does, in whether the value can hold a line
        // break at all, and in what a screen reader announces. Its height
        // is a style like every other height, so there is no `rows`
        // argument to disagree with `height is …`.
        "TextArea" => Shape {
            tag: "textarea",
            slot: Slot::Value,
            children: false,
            arguments: &["hint"],
            ..PLAIN
        },
        // A masked field, and the secrecy question it asks.
        //
        // # What secrecy the binding carries, and why it is not `Secret`
        //
        // The lattice is two-point (§5.3) and its `Secret` means "must not
        // become visible to the browser". `zdc-graph` therefore refuses
        // `secret` on a `client` placement outright, with E-IFC-01, on the
        // ground that client state is the browser's own memory. A value a
        // reader types into their own browser is already there. Labelling
        // it `Secret` would make the declaration itself the violation, so
        // every program using this element would be refused: the label
        // would be false the moment it was applied.
        //
        // So the binding is an ordinary `client Text`, labelled `Public`
        // like every other client signal, and this element is **not** a
        // route to a `secret`. That is the decision, and it is stated
        // rather than inherited.
        //
        // # What is enforced instead, and where
        //
        // The lattice's question is "may this value reach that sink". The
        // sinks a *view* can reach with a password are exactly two: it can
        // be shown, and it can be put in a URL-bearing attribute, which is
        // the class that produced a working exfiltration in this
        // repository. Both are refused, by `check_masked` in `view.rs`,
        // under one rule that covers them and everything like them: **the
        // signal a `PasswordInput` binds may appear in the view as that
        // field's own binding and nowhere else.** A second field bound to
        // the same signal is refused too, because an unmasked mirror of a
        // masked field is the echo with extra steps.
        //
        // What is deliberately *not* refused is a handler sending it
        // somewhere. That is what a password is for, and the rules over
        // that path are §14B.5's placement rule and the flow pass, which
        // already exist and already range over it.
        //
        // The three baked attributes are what the browser gives and
        // nothing else does. `autocomplete` names the field for a password
        // manager, which is what stops readers choosing a password they
        // can retype; `spellcheck="false"` keeps the value out of the
        // dictionary a spell checker builds, and out of the network
        // request some of them make.
        "PasswordInput" => Shape {
            tag: "input",
            attributes: &[
                ("type", "password"),
                ("autocomplete", "current-password"),
                ("spellcheck", "false"),
            ],
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
        // Disclosure, from the browser rather than from the program.
        //
        // `examples/disclosure.zd` built one out of a component with its
        // own `state` and an `if`, which is a fine demonstration of
        // components and a poor way to get disclosure: it costs a signal
        // and a conditional region per panel, and it gets none of what the
        // native element gives. `details` is focusable and operable from
        // the keyboard without a handler, expands when find-in-page lands
        // inside it, is announced as expanded or collapsed, and prints
        // open.
        //
        // No `open` argument and no binding, deliberately. The element
        // owns its own state, and a two-way binding to it would be a
        // second place that state lives, so a program could write one
        // value and the browser another. A disclosure whose openness the
        // program must control is an `if`, which the language already has.
        //
        // `Summary` is required and first, for the reason `Fieldset`
        // requires a `Legend`: a `details` with no summary is labelled
        // with whatever word the browser chose, in whatever language it
        // chose it in.
        "Details" => Shape {
            tag: "details",
            leading_child: Some("Summary"),
            ..PLAIN
        },
        "Summary" => Shape {
            tag: "summary",
            slot: Slot::Text,
            children: false,
            only_inside: &["Details"],
            ..PLAIN
        },
        // A set of controls that answer one question, announced as one
        // thing. A radio group is the case that cannot be done any other
        // way: without a `fieldset` a screen reader reads each radio's own
        // label and never says what the choice is about.
        //
        // The `Legend` is required and must come first, which is also
        // what HTML's own content model says. The reason to check it
        // rather than trust it is that a `fieldset` with a misplaced
        // legend is announced as an unnamed group, which is worse than no
        // grouping at all: every control inside gains the word "group"
        // and none of them gains a subject.
        "Fieldset" => Shape {
            tag: "fieldset",
            leading_child: Some("Legend"),
            ..PLAIN
        },
        "Legend" => Shape {
            tag: "legend",
            slot: Slot::Text,
            children: false,
            only_inside: &["Fieldset"],
            ..PLAIN
        },
        // A control's accessible name, associated explicitly.
        //
        // `Checkbox label is …` wraps the box in a `<label>`, which
        // handles the one case where the name is short and sits beside a
        // box. Every other control had nowhere to carry a name, so the
        // association assistive technology depends on was either implicit
        // — proximity, which is not an association — or absent.
        //
        // Explicit and by id rather than by wrapping, and that is the
        // decision. Wrapping works only when the label is next to the
        // control in the tree, so it cannot name a control in another
        // column of a form, cannot name one written inside an `if`, and
        // cannot be written at all where the two are laid out separately.
        // `for` against `id` has none of those limits, and `id` is
        // already a global argument, so nothing new is needed on the
        // control's side.
        //
        // `controls` is required for the same reason `Image` requires
        // `alt`: a `<label>` with no `for` names nothing, and it looks
        // exactly like one that does.
        "Label" => Shape {
            tag: "label",
            slot: Slot::Text,
            children: false,
            arguments: &["controls"],
            required_arguments: &["controls"],
            ..PLAIN
        },
        "Spinner" => Shape {
            tag: "span",
            attributes: &[("aria-busy", "true")],
            children: false,
            literal_text: Some("…"),
            ..PLAIN
        },
        // Completion toward a goal. `Spinner` covers the indeterminate
        // case and nothing covered the determinate one, so an upload, a
        // multi-step form and a long derivation had no way to show what
        // they had done.
        //
        // The value is one way. A progress bar is a report and not a
        // control, so there is no listener and no §14B.5 rule to apply,
        // which is also why the value may be any numeric expression rather
        // than having to be a `state` name.
        //
        // `most` and not `max`: the goal is what the number counts up to,
        // and the default of 1 makes `Progress fraction` read as a
        // fraction, which is what a program that has one already has.
        //
        // `label` becomes `aria-label` here rather than being consumed, as
        // it is on `Checkbox`. The two meanings are the same sentence,
        // what this control is called, reaching the accessibility tree by
        // the only route each element has: a checkbox can be wrapped in a
        // `<label>` and a `progress` cannot usefully be, because there is
        // no text beside it to wrap.
        "Progress" => Shape {
            tag: "progress",
            slot: Slot::Amount,
            children: false,
            arguments: &["most", "label"],
            ..PLAIN
        },
        // A measurement inside a range, which is not a progress bar and is
        // read differently: `progress` says how far a task has got and
        // `meter` says where a value sits. Disk space, a score, a battery,
        // a load average.
        //
        // `low`, `high` and `best` are what a browser colours the bar by,
        // and they are the reason this element earns its own name: they
        // say which end is good, which a bar drawn out of a `Row` and a
        // width cannot say to anybody who is not looking at it.
        "Meter" => Shape {
            tag: "meter",
            slot: Slot::Amount,
            children: false,
            arguments: &["least", "most", "low", "high", "best", "label"],
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
pub const GLOBAL_ARGUMENTS: &[&str] = &["class", "id", "title", "role", "lang", "hidden"];

/// One style argument: the CSS property it writes, and what it admits.
///
/// The grammar is the decision each of these carries. A property whose
/// value is a closed set of words takes a [`Grammar::Keyword`] and the
/// words read as English rather than as CSS. A program writes
/// `decoration is "struck"`, not `text-decoration-line is
/// "line-through"`, because the table is
/// where CSS's own vocabulary is translated, and a program that had to
/// spell the CSS anyway would be a program that could have written the
/// CSS.
#[derive(Debug, Clone, Copy)]
pub struct StyleArgument {
    pub property: &'static str,
    pub grammar: Grammar,
    /// Printed after the value. One argument uses it: `border`, whose
    /// width alone renders nothing, because a border with no style is not
    /// drawn.
    pub suffix: Option<&'static str>,
    /// When the declaration applies, before any prefix on the argument
    /// name is considered.
    ///
    /// One argument sets it: `transition`, whose declarations exist only
    /// inside `prefers-reduced-motion: no-preference`. That is what makes
    /// the preference respected *by construction* rather than by every
    /// program remembering.
    pub condition: Condition,
}

const fn style(property: &'static str, grammar: Grammar) -> StyleArgument {
    StyleArgument {
        property,
        grammar,
        suffix: None,
        condition: Condition::Always,
    }
}

/// Every style argument, and what each admits.
///
/// The set is closed for the same reason the attribute set above is: a
/// name the compiler does not know would otherwise become a CSS property
/// of that name, and `behavior` was one once. Adding a property is adding
/// a row here, and each row is a decision about a value grammar rather
/// than a mechanism.
pub const STYLE_ARGUMENTS: &[(&str, StyleArgument)] = &[
    // Inherited from before this vocabulary existed. `padding` was a bare
    // number of pixels and still is; `weight` took anything the character
    // allowlist admitted and still does, because narrowing it would refuse
    // programs that compile today and no issue asked for it.
    ("padding", style("padding", Grammar::Lengths)),
    ("weight", style("font-weight", Grammar::Free)),
    ("color", style("color", Grammar::Colour)),
    // A colour and an image are two arguments, not one. `background is
    // "/a.png"` would have to be told apart from `background is "red"` by
    // looking at the text, and guessing which of two things a string is
    // is the decision a closed grammar exists to avoid.
    ("background", style("background-color", Grammar::Colour)),
    ("backdrop", style("background-image", Grammar::Url)),
    ("margin", style("margin", Grammar::Lengths)),
    // A width alone draws nothing, because `border-style` defaults to
    // `none`. Declaring `solid` with the width is what makes `border is 1`
    // mean what a reader thinks it means; `borderStyle` sorts after
    // `border` and overrides it.
    (
        "border",
        StyleArgument {
            property: "border",
            grammar: Grammar::Lengths,
            suffix: Some("solid"),
            condition: Condition::Always,
        },
    ),
    ("borderColor", style("border-color", Grammar::Colour)),
    (
        "borderStyle",
        style("border-style", Grammar::Keyword(BORDER_STYLES)),
    ),
    ("radius", style("border-radius", Grammar::Lengths)),
    ("display", style("display", Grammar::Keyword(DISPLAYS))),
    // Three arguments, not the `flex` shorthand. The shorthand's one-value
    // form means `1 1 0%` for a bare number and `1 1 10px` for a length,
    // so what it says depends on whether the value carries a unit, and a
    // grammar whose meaning turns on that is a grammar nobody can read.
    ("grow", style("flex-grow", Grammar::Number)),
    ("shrink", style("flex-shrink", Grammar::Number)),
    ("basis", style("flex-basis", Grammar::Lengths)),
    (
        "justify",
        style("justify-content", Grammar::Keyword(JUSTIFICATIONS)),
    ),
    ("align", style("align-items", Grammar::Keyword(ALIGNMENTS))),
    ("gap", style("gap", Grammar::Lengths)),
    ("width", style("width", Grammar::Lengths)),
    ("height", style("height", Grammar::Lengths)),
    ("minWidth", style("min-width", Grammar::Lengths)),
    ("maxWidth", style("max-width", Grammar::Lengths)),
    ("minHeight", style("min-height", Grammar::Lengths)),
    ("maxHeight", style("max-height", Grammar::Lengths)),
    ("font", style("font-family", Grammar::Keyword(FONTS))),
    ("size", style("font-size", Grammar::Keyword(TEXT_SIZES))),
    // Unitless, and that is the whole decision. `line-height: 1.6` is a
    // multiple of the element's own font size and is inherited as the
    // multiple; `line-height: 24px` is inherited as 24px, so a child at a
    // different size gets lines that overlap or float apart. The grammar
    // admits only the form that survives inheritance.
    ("lineHeight", style("line-height", Grammar::Number)),
    (
        "textAlign",
        style("text-align", Grammar::Keyword(TEXT_ALIGNMENTS)),
    ),
    (
        "decoration",
        style("text-decoration-line", Grammar::Keyword(DECORATIONS)),
    ),
    ("overflow", style("overflow", Grammar::Keyword(OVERFLOWS))),
    ("position", style("position", Grammar::Keyword(POSITIONS))),
    ("top", style("top", Grammar::Lengths)),
    ("right", style("right", Grammar::Lengths)),
    ("bottom", style("bottom", Grammar::Lengths)),
    ("left", style("left", Grammar::Lengths)),
    // `layer`, not `zIndex`. What the number means is which layer the
    // element is on, and `z-index` names the axis rather than the thing.
    // Whole numbers only: `z-index: 1.5` is not half a layer, it is an
    // invalid declaration a browser drops.
    ("layer", style("z-index", Grammar::Whole)),
    // A percentage, not a fraction. `opacity is 50` reads as half and
    // `opacity is 0.5` reads as a typo for 5, and a reader should not have
    // to know which scale a number is on to know what it says.
    ("opacity", style("opacity", Grammar::Percent)),
    ("shadow", style("box-shadow", Grammar::Keyword(SHADOWS))),
    ("cursor", style("cursor", Grammar::Keyword(CURSORS))),
    // The one argument whose declarations are conditioned by the table
    // rather than by a prefix on its name.
    (
        "transition",
        StyleArgument {
            property: "transition",
            grammar: Grammar::Keyword(TRANSITIONS),
            suffix: None,
            condition: Condition::Motion,
        },
    ),
];

/// How long a change takes, as three durations rather than as a CSS
/// transition value.
///
/// # What is being animated, and why that is not a list
///
/// `all`, deliberately. The alternative is a property list, spelled
/// `transition is "color, background-color"`, and that is a CSS
/// declaration value
/// written by the program, which is the thing this whole vocabulary
/// exists so that nobody has to write. It is also a list whose entries
/// are CSS property names, so it would reintroduce CSS's own vocabulary
/// at the one argument that had most successfully hidden it.
///
/// The cost of `all` is real and worth stating: a browser transitioning
/// `all` will animate a property the program did not mean to animate.
/// What it cannot do is animate one that is not in the folded class,
/// which is a much smaller set than a hand-written stylesheet's.
///
/// # Whether this is an effect
///
/// #99 asks, because an animation is a side effect over time and this
/// language models effects carefully everywhere else. The answer is that
/// this is not one, and the reason is not a technicality: a transition
/// declared here allocates nothing, runs no code the program wrote,
/// creates no signal, and cannot be observed by anything in the program.
/// It is a property of a class, exactly as a colour is, and the browser
/// interpolates it because it is a browser.
///
/// What would be an effect is an animation with a timeline a program can
/// start, stop, or ask about: `on animationEnd`, a `playing` signal, a
/// `then` after it finishes. None of that is expressible, and this
/// argument deliberately does not begin to make it so. When that lands it
/// will need the effect discipline; a transition does not, and pretending
/// it did would be modelling ceremony rather than effects.
const TRANSITIONS: &[(&str, &str)] = &[
    ("fast", "all 120ms ease"),
    ("medium", "all 200ms ease"),
    ("slow", "all 320ms ease"),
];

/// The pointer shapes worth naming.
///
/// Six, out of CSS's thirty-odd. The rest are resize handles for a
/// resizing interaction this language has no way to express, and `url(…)`
/// cursors, which are an image and therefore a request, one that would
/// have to go through the same sink `backdrop` does for the same reason.
/// Nothing has asked for either.
///
/// `pointer` is here even though `Button` already shows one by default,
/// because a `Row` that behaves like a button needs to say so.
const CURSORS: &[(&str, &str)] = &[
    ("pointer", "pointer"),
    ("text", "text"),
    ("wait", "wait"),
    ("move", "move"),
    ("help", "help"),
    ("notAllowed", "not-allowed"),
];

/// Elevation, as four named heights rather than as a shadow.
///
/// A `box-shadow` value is four lengths, a colour and an optional keyword,
/// and writing one is the part of CSS that people copy from a generator
/// because getting it right by hand is a craft. Naming the *heights* is
/// what the argument is for: a card is `low`, a menu is `medium`, a modal
/// is `high`, and the four values below are consistent with each other in
/// a way that four hand-written shadows on one page never are.
///
/// The values carry `rgba(…)`, which is a function call, permitted here
/// and refused in [`Grammar::Colour`] for the reason that matters: these
/// are `&'static str` in the compiler and a colour is text from a program.
/// The parenthesis is only dangerous when something else can choose it.
const SHADOWS: &[(&str, &str)] = &[
    ("none", "none"),
    ("low", "0 1px 2px rgba(0, 0, 0, 0.08)"),
    ("medium", "0 2px 8px rgba(0, 0, 0, 0.12)"),
    ("high", "0 8px 24px rgba(0, 0, 0, 0.16)"),
];

/// How an element is placed.
///
/// `static` is not in the list, and could not be: it is the placement
/// keyword, so a program writing `position is "static"` would be using one
/// of the language's own words for something else entirely. It is also the
/// default, so the way to say it is to write no `position` at all.
///
/// The four here are the four that do something. `sticky` and `fixed` are
/// what #94 asked for; `relative` is what makes a `sticky` ancestor's
/// offsets mean anything and what an `absolute` child is placed against,
/// so leaving either of the pair out would leave the other half working
/// only by accident.
const POSITIONS: &[(&str, &str)] = &[
    ("sticky", "sticky"),
    ("fixed", "fixed"),
    ("relative", "relative"),
    ("absolute", "absolute"),
];

/// What a box does with content taller or wider than itself.
///
/// `clip` rather than CSS's `hidden`, because `hidden` is already an
/// argument on every element and it means something else: `hidden is yes`
/// takes an element out of the page and out of the accessibility tree,
/// while `overflow is "clip"` cuts content off and leaves it unreachable.
/// One word for both would be the worst kind of near-synonym.
///
/// `automatic` rather than `auto` for the same reason the distribution
/// words are English: `auto` is CSS's abbreviation, and the vocabulary
/// spells things out.
const OVERFLOWS: &[(&str, &str)] = &[
    ("scroll", "scroll"),
    ("clip", "hidden"),
    ("visible", "visible"),
    ("automatic", "auto"),
];

/// The line drawn through or under text.
///
/// `struck`, not `line-through`: the CSS name describes the drawing and
/// the English one describes the meaning, and the meaning is what a todo
/// list is expressing. `underline` keeps its name because that word is
/// already English.
///
/// The property is `text-decoration-line` rather than the
/// `text-decoration` shorthand, because the shorthand also resets colour,
/// style and thickness, so `decoration is "underline"` on a `Link` would
/// silently discard a `text-decoration-color` set anywhere else, and an
/// argument that quietly unsets three things it never mentions is an
/// argument that cannot be reasoned about locally.
const DECORATIONS: &[(&str, &str)] = &[
    ("underline", "underline"),
    ("struck", "line-through"),
    ("none", "none"),
];

/// Where the lines of a block sit.
///
/// `start` and `end` rather than `left` and `right`, because the document
/// has a `lang` argument and an Arabic or Hebrew page reads the other way.
/// `left` and `right` would be correct in English and silently wrong
/// there, which is the kind of wrong nobody notices until a reader
/// complains.
///
/// Named `textAlign` and not `align` because `align` is already the
/// cross-axis distribution of a flex container's children. They are
/// different questions about different things, and one word for both
/// would make `Row align is "center"` mean two things at once.
const TEXT_ALIGNMENTS: &[(&str, &str)] = &[
    ("start", "start"),
    ("end", "end"),
    ("center", "center"),
    ("justify", "justify"),
];

/// The type scale, named rather than measured.
///
/// A free number would let every use site invent its own size, which is
/// how a document ends up with `13px`, `13.5px` and `14px` doing the same
/// job. The right-hand side is a custom property `base.css` declares, so
/// the scale is one thing in one place and a program can retune it from an
/// `assets/*.css` without touching a use site.
const TEXT_SIZES: &[(&str, &str)] = &[
    ("tiny", "var(--zd-text-tiny)"),
    ("small", "var(--zd-text-small)"),
    ("normal", "var(--zd-text-normal)"),
    ("large", "var(--zd-text-large)"),
    ("huge", "var(--zd-text-huge)"),
    ("giant", "var(--zd-text-giant)"),
];

/// The four typefaces, as stacks the compiler writes.
///
/// A program cannot name a family directly, and that is the decision
/// rather than an omission. A family name is arbitrary text that ends up
/// in a printed declaration, it needs quoting the moment it contains a
/// space, and a quoted value inside a printed rule is the shape of all
/// three injection holes this compiler has had. Four words cover what a
/// document needs, and a fifth typeface is a font file, which is a
/// different mechanism: `assets/` already copies one, and an
/// `assets/*.css` carrying `@font-face` is linked after the generated
/// sheet (see `assets.rs`).
///
/// Every stack is written without a space inside any family name, so no
/// entry here needs quoting either.
const FONTS: &[(&str, &str)] = &[
    ("system", "system-ui, sans-serif"),
    ("sans", "ui-sans-serif, system-ui, sans-serif"),
    ("serif", "ui-serif, Georgia, serif"),
    ("mono", "ui-monospace, SFMono-Regular, Menlo, monospace"),
];

/// Distribution along the direction the container runs.
///
/// The words are what a person says; CSS's own `flex-start` and
/// `space-between` are the right-hand column and stay there. A program
/// that had to write the CSS spelling would be writing CSS with extra
/// steps, and admitting both would be the two-phrasings problem §4.1
/// forbids.
const JUSTIFICATIONS: &[(&str, &str)] = &[
    ("start", "flex-start"),
    ("end", "flex-end"),
    ("center", "center"),
    ("between", "space-between"),
    ("around", "space-around"),
    ("evenly", "space-evenly"),
];

/// Distribution across the direction the container runs.
const ALIGNMENTS: &[(&str, &str)] = &[
    ("start", "flex-start"),
    ("end", "flex-end"),
    ("center", "center"),
    ("stretch", "stretch"),
    ("baseline", "baseline"),
];

/// How an element flows.
///
/// `flex` is deliberately absent. `Row` and `Column` *are* the flex
/// containers, and a second way to make one is the two-phrasings problem
/// §4.1 forbids; a `display is "flex"` on anything else would also be a
/// flex container with no way to say which direction it runs in, since
/// direction comes from the element's own base class.
///
/// `none` is here even though `hidden` exists, and they are not the same
/// thing: `hidden` is an attribute that takes the element out of the
/// accessibility tree as well as the layout, which is what a program
/// usually wants; `display is "none"` is the one a breakpoint reaches for,
/// where the element is still in the document at another width.
const DISPLAYS: &[(&str, &str)] = &[
    ("block", "block"),
    ("inline", "inline"),
    ("inlineBlock", "inline-block"),
    ("none", "none"),
];

/// The border styles worth having.
///
/// `groove`, `ridge`, `inset` and `outset` are the bevelled borders of
/// 1996 and render differently in every engine; `hidden` differs from
/// `none` only inside a table's border collapsing, which this language has
/// no way to reach.
const BORDER_STYLES: &[(&str, &str)] = &[
    ("solid", "solid"),
    ("dashed", "dashed"),
    ("dotted", "dotted"),
    ("double", "double"),
    ("none", "none"),
];

/// The prefixes that put a style argument in a circumstance.
///
/// `hoverBackground is "grey"` and `narrowDisplay is "none"` rather than a
/// nested block, because an argument list is the one place the grammar
/// already lets an element say something about itself. A block would need
/// a production of its own, and §4.1 would then count two ways to write a
/// style.
///
/// The set is closed and small. `:visited` is absent because it leaks
/// browsing history and every engine restricts what it can set;
/// `:nth-child` and friends are absent because they are selectors over
/// siblings, and an argument on one element cannot say anything about its
/// siblings without the compiler knowing what the siblings are. `narrow`
/// and `wide` are two names for one breakpoint rather than an arbitrary
/// query, for the reason [`crate::style::BREAKPOINT`] gives.
pub const PREFIXES: &[(&str, Condition)] = &[
    ("hover", Condition::Hover),
    ("focus", Condition::Focus),
    ("active", Condition::Active),
    ("disabled", Condition::Disabled),
    ("narrow", Condition::Narrow),
    ("wide", Condition::Wide),
    ("dark", Condition::Dark),
];

/// The style argument called `name`, or `None`.
///
/// A prefixed name resolves to the argument it prefixes, with the
/// prefix's circumstance replacing the argument's own. An argument that
/// *has* a circumstance of its own cannot be prefixed, because there is
/// one condition on a declaration and the prefix would silently discard
/// the other: `hoverTransition` would be a transition outside
/// `prefers-reduced-motion`, which is the one property #99 promised
/// nothing could produce.
pub fn style_argument(name: &str) -> Option<StyleArgument> {
    if let Some(plain) = plain_style_argument(name) {
        return Some(plain);
    }
    let (condition, base) = prefixed(name)?;
    let argument = plain_style_argument(&base)?;
    if argument.condition != Condition::Always {
        return None;
    }
    Some(StyleArgument {
        condition,
        ..argument
    })
}

fn plain_style_argument(name: &str) -> Option<StyleArgument> {
    STYLE_ARGUMENTS
        .iter()
        .find(|(argument, _)| *argument == name)
        .map(|(_, style)| *style)
}

/// `("hoverBackground")` becomes `(Hover, "background")`.
///
/// The remainder must start with an upper-case letter, so `hovercraft`
/// does not read as a prefixed `craft` and a base argument that happens to
/// start with a prefix's letters is not shadowed.
fn prefixed(name: &str) -> Option<(Condition, String)> {
    for (prefix, condition) in PREFIXES {
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        let mut characters = rest.chars();
        let Some(first) = characters.next() else {
            continue;
        };
        if !first.is_ascii_uppercase() {
            continue;
        }
        return Some((
            *condition,
            format!("{}{}", first.to_ascii_lowercase(), characters.as_str()),
        ));
    }
    None
}

/// Whether `element` accepts the named argument `name`.
pub fn accepts_argument(shape: &Shape, name: &str) -> bool {
    GLOBAL_ARGUMENTS.contains(&name)
        || shape.arguments.contains(&name)
        || style_argument(name).is_some()
}

/// How a named argument reaches the DOM, per `props()` in `elements.js`.
pub enum Named {
    /// A CSS declaration, folded into the element's generated class.
    Style(StyleArgument),
    /// A DOM attribute under a possibly different name.
    Attribute(&'static str),
    /// A DOM attribute holding a URL, which is filtered before it is set.
    Url(&'static str),
    /// Appended to the element's base class.
    Class,
    /// Read by the element itself and never written to the DOM.
    Consumed,
}

/// The DOM meaning of a permitted named argument on `element`.
///
/// Total over `GLOBAL_ARGUMENTS`, `STYLE_ARGUMENTS` and every
/// `Shape::arguments` entry, which `named_arguments_are_total` below
/// checks; `accepts_argument` has already rejected everything else.
///
/// The element is a parameter because two names mean different things on
/// different elements, and both meanings are right. See the `width` arm.
pub fn named_argument(element: &str, name: &str) -> Option<Named> {
    // `Image` and `Canvas` size themselves through *attributes*. An `img`
    // with `width` and `height` reserves its layout box before the file
    // arrives, which is what stops a page reflowing as images load, and no
    // stylesheet rule can do that because the rule does not know the
    // aspect ratio. Everywhere else a width is a style, so the two
    // meanings are the same sentence, how wide it is, reaching the
    // browser by the only route that works for each.
    if matches!(name, "width" | "height")
        && matches!(element, "Image" | "Canvas" | "Video" | "Frame")
    {
        return Some(Named::Attribute(match name {
            "width" => "width",
            _ => "height",
        }));
    }
    // `label` is the second name whose meaning depends on the element,
    // and both meanings are the same sentence: what this control is
    // called. `Checkbox` wraps the box in a `<label>` and consumes it;
    // `Progress` and `Meter` have no text beside them to wrap, so the name
    // reaches the accessibility tree as an attribute instead.
    if name == "label" && matches!(element, "Progress" | "Meter") {
        return Some(Named::Attribute("aria-label"));
    }
    if let Some(argument) = style_argument(name) {
        return Some(Named::Style(argument));
    }
    let named = match name {
        "class" => Named::Class,
        "hint" => Named::Attribute("placeholder"),
        "exact" => Named::Attribute("datetime"),
        "expansion" => Named::Attribute("title"),
        // `for` is a Rust keyword and a JavaScript one, and it reads as a
        // preposition rather than as a claim. `controls is "email-field"`
        // says what the label does.
        "controls" => Named::Attribute("for"),
        "source" => Named::Url("src"),
        // The still a video shows before it plays. A request the browser
        // issues at once, so it takes the filtered path `source` does.
        "poster" => Named::Url("poster"),
        "id" => Named::Attribute("id"),
        "title" => Named::Attribute("title"),
        "role" => Named::Attribute("role"),
        "lang" => Named::Attribute("lang"),
        "hidden" => Named::Attribute("hidden"),
        "alt" => Named::Attribute("alt"),
        "loading" => Named::Attribute("loading"),
        // The ends and the landmarks of a measured range, in English. CSS
        // and HTML call them `min`, `max`, `low`, `high` and `optimum`;
        // `least` and `most` say what they are without abbreviating, and
        // `best` says what `optimum` means.
        "least" => Named::Attribute("min"),
        "most" => Named::Attribute("max"),
        "low" => Named::Attribute("low"),
        "high" => Named::Attribute("high"),
        "best" => Named::Attribute("optimum"),
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

/// Whether a compile-time-known style value may be folded into the
/// generated stylesheet.
///
/// `padding is …` and `weight is …` fold into a rule in `styles.css`
/// (§16.3.11), and a declaration there is written `property: value;`
/// inside braces. `Styles::stylesheet` *prints* that pair, so unlike
/// `bindStyle` — which hands one declaration to `setProperty` and has the
/// CSSOM drop anything after it — a value here is not confined to its
/// declaration. `weight is "bold; } body { display: none } x {"` is a
/// rule for `body`, which is a defacement of the whole page; `url(…)` in
/// one is an outbound request the program never wrote, and `/*` swallows
/// the rest of the sheet.
///
/// The same argument §16.3.5 makes about markup applies here and reaches
/// the opposite conclusion, because there is no CSS escape that keeps the
/// value meaning what it said: an escaped `;` is not a semicolon in a
/// length. So the set is closed rather than escaped, exactly as the
/// argument set is — a length, a keyword, a colour, a comma-separated
/// list of those, and nothing that can end a declaration.
///
/// **It is an allowlist, and deliberately.** An earlier form of this
/// check named the characters it refused. That shape has to grow a new
/// entry every time the surface language makes a new character reachable
/// — a line break had to be added to it by hand once block text literals
/// landed, because a value could suddenly carry one, and a declaration
/// printed across four lines of a generated stylesheet is not a style
/// anybody wrote on purpose. Naming what a style value *is* costs the
/// next such feature nothing: a character nobody has considered is
/// refused until somebody considers it.
///
/// The reactive path needs no rule: `bindStyle` reaches the value through
/// `style.setProperty`, which parses one declaration and drops it whole if
/// it does not parse. Only the folded literal is written into a sheet.
pub fn style_value_is_permitted(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '%' | '#' | ',' | '+')
        })
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
        assert!(matches!(
            named_argument("Image", "source"),
            Some(Named::Url("src"))
        ));
    }

    /// The values a program writes, and the ones that would stop being a
    /// value the moment the rule is printed.
    #[test]
    fn a_style_value_may_not_end_the_declaration_it_sits_in() {
        for permitted in [
            "bold",
            "8px",
            "0.5rem",
            "100%",
            "#b3151c",
            "1px solid #ccc",
            "700",
        ] {
            assert!(
                style_value_is_permitted(permitted),
                "`{permitted}` is a style value a program writes"
            );
        }
        for refused in [
            "",
            "bold; } * { display: none } .z {",
            "red } body { display:none",
            "url(https://example.com/x.png)",
            "bold /* swallow the rest",
            "red;",
            "\"quoted\"",
            "red\n}",
        ] {
            assert!(
                !style_value_is_permitted(refused),
                "`{refused}` can end the declaration it is written into"
            );
        }
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
        // `Column` stands for "any element", because a global argument is
        // one whose meaning does not depend on the element. The two names
        // whose meaning does, `width` and `height`, are checked against
        // both kinds of element below.
        for name in GLOBAL_ARGUMENTS {
            scanned += 1;
            assert!(
                named_argument("Column", name).is_some(),
                "`{name}` is accepted everywhere but has no DOM meaning"
            );
        }
        for (name, argument) in STYLE_ARGUMENTS {
            scanned += 1;
            assert!(
                matches!(named_argument("Column", name), Some(Named::Style(_))),
                "`{name}` is a style argument that does not reach the stylesheet"
            );
            // The prefixed spellings are accepted by `accepts_argument`,
            // so each of them needs a meaning too, or a program would be
            // told `hoverColor` is fine and then told it has no meaning.
            for (prefix, condition) in PREFIXES {
                let mut characters = name.chars();
                let first = characters.next().expect("an argument name is not empty");
                let prefixed = format!(
                    "{prefix}{}{}",
                    first.to_ascii_uppercase(),
                    characters.as_str()
                );
                scanned += 1;
                let column = shape("Column").expect("Column");
                if argument.condition != Condition::Always {
                    // `transition` carries its own circumstance, and a
                    // declaration has one condition: a prefix would
                    // discard the motion query silently.
                    assert!(
                        !accepts_argument(&column, &prefixed),
                        "`{prefixed}` would drop `{name}`'s own condition"
                    );
                    continue;
                }
                assert!(
                    accepts_argument(&column, &prefixed),
                    "`{prefixed}` is spelled from a prefix and a style argument"
                );
                assert!(
                    matches!(
                        named_argument("Column", &prefixed),
                        Some(Named::Style(StyleArgument { condition: c, .. })) if c == *condition
                    ),
                    "`{prefixed}` must apply `{name}` in the `{prefix}` state alone"
                );
            }
        }
        for element in BUILT_INS {
            let shape = shape(element).expect("a built-in has a shape");
            for name in shape.arguments {
                scanned += 1;
                assert!(
                    named_argument(element, name).is_some(),
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
            named_argument("Column", "padding"),
            Some(Named::Style(StyleArgument {
                property: "padding",
                grammar: Grammar::Lengths,
                ..
            }))
        ));
        assert!(matches!(
            named_argument("Column", "weight"),
            Some(Named::Style(StyleArgument {
                property: "font-weight",
                grammar: Grammar::Free,
                ..
            }))
        ));
        assert!(matches!(
            named_argument("Input", "hint"),
            Some(Named::Attribute("placeholder"))
        ));
        assert!(matches!(
            named_argument("ErrorBar", "message"),
            Some(Named::Consumed)
        ));
        assert!(matches!(
            named_argument("Column", "id"),
            Some(Named::Attribute("id"))
        ));
    }

    /// A colour is a style everywhere, and there is no element on which it
    /// is anything else.
    #[test]
    fn a_colour_is_a_folded_declaration_and_not_an_attribute() {
        let mut checked = 0;
        for element in BUILT_INS {
            checked += 1;
            assert!(
                matches!(
                    named_argument(element, "color"),
                    Some(Named::Style(StyleArgument {
                        property: "color",
                        grammar: Grammar::Colour,
                        ..
                    }))
                ),
                "`{element}` gives `color` some other meaning"
            );
        }
        assert_eq!(checked, BUILT_INS.len());
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
