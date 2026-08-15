//! The HIR node types.
//!
//! Every node carries the span of the source it came from: later passes
//! report their errors against HIR rather than AST, so a node without a
//! span is a diagnostic that cannot point anywhere.

use crate::ids::{Arena, ArenaId, BlockId, DefId, ExprId, LocalId, PlaceId};
use zdc_lexer::Span;

/// What a resolved name points at.
///
/// After resolution no reference is a string. Later passes match on one
/// of these variants instead of on spelling, so a rename cannot silently
/// change which declaration a pass believes it is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Res {
    /// A top-level `state`, `function`, `record`, `choice`, or `view`.
    Def(DefId),
    /// A parameter, loop variable, or pattern binding.
    Local(LocalId),
    /// A name the language provides rather than the program.
    Builtin(Builtin),
    /// One variant of a choice the language provides — `Some`, `None`,
    /// `Loading`, `Ready`, `Failed`.
    ///
    /// §17.4.2: `BUILTIN_PATTERNS` recognised these in *pattern* position
    /// only, so no function could ever return an `Option`. A library that
    /// cannot write `Some with value is v` cannot be written at all, which
    /// is what this variant fixes.
    BuiltinVariant(BuiltinVariant),
    /// One variant of a user-declared `choice`, by the choice it belongs to
    /// and its position in the declaration.
    ///
    /// A variant name is a value (`All`) or a constructor (`Archived with
    /// reason is "old"`), and both need the choice as well as the name, so
    /// resolution settles it here rather than leaving codegen to search.
    Variant { choice: DefId, index: u32 },
}

/// The kind of built-in a `Res::Builtin` names.
///
/// A stopgap until user-defined components (spec §14D) and record and
/// choice declarations (§14B.1) exist, at which point built-in elements
/// and types become ordinary definitions with a `DefId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    /// A view element the language provides, such as `Row` or `Text`.
    Element(BuiltinElement),
    /// A type name the language provides, such as `Text` or `Whole`.
    Type,
    /// `Pair with first is …, second is …`: the one constructor the
    /// language provides that is not a variant of a choice.
    ///
    /// It is here rather than in [`BuiltinVariant`] because a pair is not
    /// a choice: nothing dispatches on it, `when` never meets one, and
    /// [`BuiltinVariant`]'s members all answer "which arm of `Option`,
    /// `Remote` or `Code` is this". A pair has no arms.
    Pair,
}

/// Which view element a [`Builtin::Element`] names (spec §17.2.2(b)).
///
/// Carrying the element rather than a bare marker is what lets a pass ask
/// "is this the two-way `Input`?" without matching on a string. A string
/// match is a live soundness hole the moment §14D lets a program declare
/// `component Input`: a user component resolves to [`Res::Def`] and can
/// never be confused with the built-in, but only if the question is asked
/// of the resolution rather than of the spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinElement {
    // layout
    Column,
    Row,
    // document structure
    Main,
    Section,
    Article,
    Aside,
    Navigation,
    Header,
    Footer,
    Address,
    Divider,
    // text
    Text,
    Heading,
    Paragraph,
    Emphasis,
    Strong,
    Code,
    CodeBlock,
    Preformatted,
    Break,
    Quote,
    Key,
    Time,
    Small,
    Mark,
    Abbreviation,
    Superscript,
    Subscript,
    // rendered documents
    Prose,
    // lists
    List,
    NumberedList,
    Item,
    Terms,
    Term,
    Description,
    Table,
    HeaderRow,
    TableRow,
    HeaderCell,
    Cell,
    // links and media
    Link,
    Image,
    Video,
    Audio,
    Frame,
    Figure,
    Caption,
    Canvas,
    // vector drawing
    Svg,
    Group,
    Path,
    Circle,
    Segment,
    Scene,
    // controls
    Button,
    Form,
    Input,
    TextArea,
    PasswordInput,
    NumberInput,
    DateInput,
    FileInput,
    Slider,
    Select,
    Radio,
    Checkbox,
    Label,
    Fieldset,
    Legend,
    Details,
    Summary,
    Dialog,
    Spinner,
    Progress,
    Meter,
    ErrorBar,
}

impl BuiltinElement {
    /// Every built-in, in the order the vocabulary is grouped.
    ///
    /// This array is the **one** table. [`BuiltinElement::NAMES`] is
    /// derived from it and `zdc-resolve` re-exports that rather than
    /// keeping a list of its own: two lists of element names is exactly
    /// the drift `scripts/check-grammar-drift.py` exists to catch, and a
    /// name present in one and absent from the other is a vocabulary that
    /// diagnoses a spelling it then refuses to resolve.
    ///
    /// The length is written out rather than inferred, so adding a
    /// variant without adding it here is a compile error rather than a
    /// quietly shorter table. `the_vocabulary_is_enumerated` below
    /// checks the same property from the enum's side.
    pub const ALL: [BuiltinElement; 76] = [
        BuiltinElement::Column,
        BuiltinElement::Row,
        BuiltinElement::Main,
        BuiltinElement::Section,
        BuiltinElement::Article,
        BuiltinElement::Aside,
        BuiltinElement::Navigation,
        BuiltinElement::Header,
        BuiltinElement::Footer,
        BuiltinElement::Address,
        BuiltinElement::Divider,
        BuiltinElement::Text,
        BuiltinElement::Heading,
        BuiltinElement::Paragraph,
        BuiltinElement::Emphasis,
        BuiltinElement::Strong,
        BuiltinElement::Code,
        BuiltinElement::CodeBlock,
        BuiltinElement::Preformatted,
        BuiltinElement::Break,
        BuiltinElement::Quote,
        BuiltinElement::Key,
        BuiltinElement::Time,
        BuiltinElement::Small,
        BuiltinElement::Mark,
        BuiltinElement::Abbreviation,
        BuiltinElement::Superscript,
        BuiltinElement::Subscript,
        BuiltinElement::Prose,
        BuiltinElement::List,
        BuiltinElement::NumberedList,
        BuiltinElement::Item,
        BuiltinElement::Terms,
        BuiltinElement::Term,
        BuiltinElement::Description,
        BuiltinElement::Table,
        BuiltinElement::HeaderRow,
        BuiltinElement::TableRow,
        BuiltinElement::HeaderCell,
        BuiltinElement::Cell,
        BuiltinElement::Link,
        BuiltinElement::Image,
        BuiltinElement::Video,
        BuiltinElement::Audio,
        BuiltinElement::Frame,
        BuiltinElement::Figure,
        BuiltinElement::Caption,
        BuiltinElement::Canvas,
        BuiltinElement::Svg,
        BuiltinElement::Group,
        BuiltinElement::Path,
        BuiltinElement::Circle,
        BuiltinElement::Segment,
        BuiltinElement::Scene,
        BuiltinElement::Button,
        BuiltinElement::Form,
        BuiltinElement::Input,
        BuiltinElement::TextArea,
        BuiltinElement::PasswordInput,
        BuiltinElement::NumberInput,
        BuiltinElement::DateInput,
        BuiltinElement::FileInput,
        BuiltinElement::Slider,
        BuiltinElement::Select,
        BuiltinElement::Radio,
        BuiltinElement::Checkbox,
        BuiltinElement::Label,
        BuiltinElement::Fieldset,
        BuiltinElement::Legend,
        BuiltinElement::Details,
        BuiltinElement::Summary,
        BuiltinElement::Dialog,
        BuiltinElement::Spinner,
        BuiltinElement::Progress,
        BuiltinElement::Meter,
        BuiltinElement::ErrorBar,
    ];

    /// The same, as the spellings a program writes.
    pub const NAMES: &'static [&'static str] = &[
        "Column",
        "Row",
        "Main",
        "Section",
        "Article",
        "Aside",
        "Navigation",
        "Header",
        "Footer",
        "Address",
        "Divider",
        "Text",
        "Heading",
        "Paragraph",
        "Emphasis",
        "Strong",
        "Code",
        "CodeBlock",
        "Preformatted",
        "Break",
        "Quote",
        "Key",
        "Time",
        "Small",
        "Mark",
        "Abbreviation",
        "Superscript",
        "Subscript",
        "Prose",
        "List",
        "NumberedList",
        "Item",
        "Terms",
        "Term",
        "Description",
        "Table",
        "HeaderRow",
        "TableRow",
        "HeaderCell",
        "Cell",
        "Link",
        "Image",
        "Video",
        "Audio",
        "Frame",
        "Figure",
        "Caption",
        "Canvas",
        "Svg",
        "Group",
        "Path",
        "Circle",
        "Segment",
        "Scene",
        "Button",
        "Form",
        "Input",
        "TextArea",
        "PasswordInput",
        "NumberInput",
        "DateInput",
        "FileInput",
        "Slider",
        "Select",
        "Radio",
        "Checkbox",
        "Label",
        "Fieldset",
        "Legend",
        "Details",
        "Summary",
        "Dialog",
        "Spinner",
        "Progress",
        "Meter",
        "ErrorBar",
    ];

    /// Whether this element writes back into the signal bound to its first
    /// positional argument on every interaction (spec §14B.5).
    ///
    /// Ten of the eleven are controls a person types in, drags or picks
    /// from, and their write is the interaction itself.
    /// [`BuiltinElement::Dialog`] is the eleventh and
    /// its write is a *dismissal*: Escape and the browser's own close
    /// request end in a `close` event, and the signal that opened the
    /// dialog is what has to learn about it. Without the write-back the
    /// program and the DOM disagree about whether the dialog is open, and
    /// the next click on the button that opened it does nothing at all.
    /// So it belongs here, and §14B.5's rule about which signals may be
    /// written this way applies to it unchanged.
    /// `FileInput` is here even though its *other* direction is missing —
    /// no script may put a file into a file picker, so the compiler can
    /// only clear one. What this predicate is asked for is the direction
    /// that exists: the browser writes the signal, so the signal has a
    /// writer. Three passes read it that way and all three need the
    /// answer to be yes. `zdc-codegen`'s `analysis` allocates the setter
    /// a binding with no `set` statement still needs; `zdc-graph`'s
    /// `sites` records a `Site::Bind`, which is what makes the cell
    /// Untrusted under G-SIG's second clause — a name the reader chose is
    /// attacker-chosen text; and `zdc-graph`'s `ifc` carries the
    /// enclosing `pc` onto that write.
    pub fn is_two_way(self) -> bool {
        matches!(
            self,
            BuiltinElement::Input
                | BuiltinElement::TextArea
                | BuiltinElement::PasswordInput
                | BuiltinElement::NumberInput
                | BuiltinElement::DateInput
                | BuiltinElement::FileInput
                | BuiltinElement::Slider
                | BuiltinElement::Select
                | BuiltinElement::Radio
                | BuiltinElement::Checkbox
                | BuiltinElement::Dialog
        )
    }

    /// The named arguments of *this* element that the browser dereferences
    /// as a URL (spec §14G.1.3(c) sink 7).
    ///
    /// **The `match` has no wildcard arm, and that is the point.** A new
    /// element cannot be added to the vocabulary without deciding, here,
    /// whether it carries a URL — which is the same lesson §16.3.10 draws
    /// about wildcard match arms in the emitter. A list a future element
    /// can silently fall through is not a closed list.
    ///
    /// This is *not* the enforcement boundary. Enforcement is
    /// [`crate::is_url_attribute`], which ranges over the attribute name
    /// on every element, because `named_argument` passes an unrecognised
    /// name through to the attribute of that name: `Text src is …` reaches
    /// the DOM whether or not `Text` was meant to have a `src`. The two
    /// are tied together by a test.
    pub fn url_arguments(self) -> &'static [&'static str] {
        match self {
            BuiltinElement::Column
            | BuiltinElement::Row
            | BuiltinElement::Main
            | BuiltinElement::Section
            | BuiltinElement::Article
            | BuiltinElement::Aside
            | BuiltinElement::Navigation
            | BuiltinElement::Header
            | BuiltinElement::Footer
            | BuiltinElement::Address
            | BuiltinElement::Divider
            | BuiltinElement::Text
            | BuiltinElement::Heading
            | BuiltinElement::Paragraph
            | BuiltinElement::Emphasis
            | BuiltinElement::Strong
            | BuiltinElement::Code
            | BuiltinElement::CodeBlock
            | BuiltinElement::Preformatted
            | BuiltinElement::Break
            | BuiltinElement::Quote
            | BuiltinElement::Key
            | BuiltinElement::Time
            | BuiltinElement::Small
            | BuiltinElement::Mark
            | BuiltinElement::Abbreviation
            | BuiltinElement::Superscript
            | BuiltinElement::Subscript
            // `Prose` carries no URL *argument*. The URLs inside the
            // `Markup` it renders were settled by `build markdown`, which
            // rewrites every non-http(s) one before the value exists, so
            // there is nothing here for a name-keyed rule to filter.
            | BuiltinElement::Prose
            | BuiltinElement::List
            | BuiltinElement::NumberedList
            | BuiltinElement::Item
            | BuiltinElement::Terms
            | BuiltinElement::Term
            | BuiltinElement::Description
            | BuiltinElement::Table
            | BuiltinElement::HeaderRow
            | BuiltinElement::TableRow
            | BuiltinElement::HeaderCell
            | BuiltinElement::Cell
            | BuiltinElement::Figure
            | BuiltinElement::Caption
            | BuiltinElement::Canvas
            // The vector family names no document. `Path`'s `outline` is a
            // path string the renderer walks, not a reference the browser
            // dereferences, and `Svg`'s `viewBox` is four numbers. The one
            // SVG element that *would* carry a URL — `<image href>` — is
            // deliberately absent: `Image` already exists and is filtered.
            | BuiltinElement::Svg
            | BuiltinElement::Group
            | BuiltinElement::Path
            | BuiltinElement::Circle
            | BuiltinElement::Segment
            | BuiltinElement::Scene
            | BuiltinElement::Button
            // A `form` has an `action`, which is URL-bearing, and this
            // vocabulary does not offer it: submission is a handler this
            // program runs, never a navigation to a URL it names.
            | BuiltinElement::Form
            | BuiltinElement::Input
            | BuiltinElement::TextArea
            | BuiltinElement::PasswordInput
            // Neither numeric field carries a URL. Both are `type`d inputs
            // whose whole value is a number the browser parsed, and
            // neither takes an argument the browser dereferences.
            | BuiltinElement::NumberInput
            | BuiltinElement::DateInput
            // A file picker carries no URL either. What a reader chose is
            // named by a `File` object the browser keeps to itself, and
            // the one thing this element hands the program is that file's
            // *name* — text, never dereferenced. The `blob:` URL a script
            // could mint from the file is not expressible here, which is
            // the point: it would be a URL-bearing argument nothing in
            // §14G.1.3's sink 7 had ruled on.
            | BuiltinElement::FileInput
            | BuiltinElement::Slider
            | BuiltinElement::Select
            | BuiltinElement::Radio
            | BuiltinElement::Checkbox
            | BuiltinElement::Label
            | BuiltinElement::Fieldset
            | BuiltinElement::Legend
            | BuiltinElement::Details
            | BuiltinElement::Summary
            // A modal has no URL of its own. It is a region of *this*
            // document that the browser puts in the top layer; nothing
            // about opening one is a request.
            | BuiltinElement::Dialog
            | BuiltinElement::Spinner
            | BuiltinElement::Progress
            | BuiltinElement::Meter
            | BuiltinElement::ErrorBar => &[],
            BuiltinElement::Image => &["source"],
            // Two, and both are requests the browser issues before
            // anything is painted: the file it plays and the still it
            // shows until then.
            BuiltinElement::Video => &["source", "poster"],
            BuiltinElement::Audio => &["source"],
            // The worst URL in the vocabulary: the document it names is
            // loaded and run in the reader's browser, inside a sandbox
            // the compiler writes and no program can widen.
            BuiltinElement::Frame => &["source"],
            // `Link`'s destination is its *leading* argument (§14G.2
            // revision 1) and would be invisible to a name-keyed rule —
            // so `zdc-resolve` lowers it under `DESTINATION_ARGUMENT`,
            // which is this name. By the time any pass sees a `Link` the
            // destination is an ordinary named argument.
            BuiltinElement::Link => &["href"],
        }
    }

    pub fn name(self) -> &'static str {
        BuiltinElement::NAMES[self as usize]
    }

    pub fn from_name(name: &str) -> Option<BuiltinElement> {
        BuiltinElement::ALL
            .into_iter()
            .find(|element| element.name() == name)
    }
}

/// One variant of `Option of T`, `Remote of T`, or `Code`.
///
/// `Code` is the type of a `Failed` payload's `code` field, and its three
/// arms are the transport outcomes `runtime/rpc.js` can produce. This
/// crate cannot see `zdc-types`, so the spellings are written out here;
/// `zdc-resolve` — which sees both — has the test that pins them to
/// `FailureCode`, and a fourth outcome added there fails it here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinVariant {
    Some,
    None,
    Loading,
    Ready,
    Failed,
    Unreachable,
    Timeout,
    Rejected,
}

impl BuiltinVariant {
    pub fn from_name(name: &str) -> Option<BuiltinVariant> {
        Some(match name {
            "Some" => BuiltinVariant::Some,
            "None" => BuiltinVariant::None,
            "Loading" => BuiltinVariant::Loading,
            "Ready" => BuiltinVariant::Ready,
            "Failed" => BuiltinVariant::Failed,
            "Unreachable" => BuiltinVariant::Unreachable,
            "Timeout" => BuiltinVariant::Timeout,
            "Rejected" => BuiltinVariant::Rejected,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            BuiltinVariant::Some => "Some",
            BuiltinVariant::None => "None",
            BuiltinVariant::Loading => "Loading",
            BuiltinVariant::Ready => "Ready",
            BuiltinVariant::Failed => "Failed",
            BuiltinVariant::Unreachable => "Unreachable",
            BuiltinVariant::Timeout => "Timeout",
            BuiltinVariant::Rejected => "Rejected",
        }
    }

    /// The names of the fields this variant carries, in declaration order.
    pub fn field_names(self) -> &'static [&'static str] {
        match self {
            BuiltinVariant::Some => &["value"],
            BuiltinVariant::Ready => &["value"],
            BuiltinVariant::Failed => &["error"],
            BuiltinVariant::None
            | BuiltinVariant::Loading
            | BuiltinVariant::Unreachable
            | BuiltinVariant::Timeout
            | BuiltinVariant::Rejected => &[],
        }
    }

    /// Every built-in variant name, for the resolver's table and for an
    /// editor offering them.
    pub const ALL: &'static [BuiltinVariant] = &[
        BuiltinVariant::Loading,
        BuiltinVariant::Ready,
        BuiltinVariant::Failed,
        BuiltinVariant::Some,
        BuiltinVariant::None,
        BuiltinVariant::Unreachable,
        BuiltinVariant::Timeout,
        BuiltinVariant::Rejected,
    ];
}

/// A whole resolved program.
#[derive(Debug, Clone, PartialEq)]
pub struct Hir {
    pub defs: Arena<DefId, Def>,
    pub locals: Arena<LocalId, Local>,
    pub exprs: Arena<ExprId, HirExpr>,
    pub blocks: Arena<BlockId, HirBlock>,
    /// The `view` declaration, if the program has one.
    pub view: Option<DefId>,
    /// How many leading definitions came from the prelude (§17.4.1).
    ///
    /// The prelude is resolved into *these* arenas rather than its own, so
    /// a user reference to `valueOr` is an ordinary `Res::Def` and every
    /// later pass needs no rule for it. It is allocated first and
    /// contiguously, so one number separates the library from the program —
    /// which is what lets an editor list only the user's declarations and
    /// lets a diagnostic tell "the user wrote this" from "the library did".
    pub prelude_defs: usize,
    /// How many leading expressions came from the prelude. Spans below
    /// this index index the prelude's own source files, not the user's.
    pub prelude_exprs: usize,
    /// How many leading binders came from the prelude.
    pub prelude_locals: usize,
    /// How many places have been handed an id.
    ///
    /// A counter rather than an arena: a place is stored inline in its
    /// statement, so nothing needs to look one up — only to tell two
    /// apart (#13).
    pub places: u32,
    /// The `route` declaration, if the program has one, and the URL each
    /// of its variants renders (spec §14G.2).
    ///
    /// A route lowers to an ordinary [`DefKind::Choice`] — it *is* a
    /// choice, plus a bijection onto URLs — so `when` dispatch, variant
    /// construction, exhaustiveness and field binding are the machinery
    /// that already exists rather than a second copy of it. This table is
    /// the bijection, and nothing else about a route is special.
    pub routes: Option<(DefId, RouteTable)>,
}

/// The URL side of a `route` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteTable {
    /// One entry per variant, in declaration order — the same order
    /// [`Choice::variants`] is in, so the two are indexed alike.
    pub variants: Vec<RouteVariantInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteVariantInfo {
    /// The literal prefix, beginning with `/`.
    pub path: String,
    pub path_span: Span,
    pub params: Vec<RouteParam>,
    pub span: Span,
}

/// One route parameter: a variant field that also appears in the URL.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteParam {
    pub name: String,
    /// The `static` signal holding every value this parameter ranges
    /// over, if it is enumerable.
    ///
    /// `None` makes the parameter **untrusted** (spec §18.1 semantics 5):
    /// nothing proved the value came from anywhere but the URL bar.
    /// `Some` makes it trusted, because a successful match against a
    /// compiler-rendered enumeration is a proof rather than a check.
    pub enumerated_in: Option<DefId>,
    pub span: Span,
}

impl RouteTable {
    /// The URL a variant renders with these parameter values.
    ///
    /// One function, used by the collision check, by `Link`, by the page
    /// emitter and by the manifest, so no two of them can disagree about
    /// what a route's URL is.
    pub fn url(&self, index: usize, values: &[String]) -> String {
        let Some(variant) = self.variants.get(index) else {
            return String::new();
        };
        let mut out = variant.path.trim_end_matches('/').to_string();
        for value in values {
            out.push('/');
            out.push_str(value);
        }
        if out.is_empty() {
            out.push('/');
        }
        out
    }
}

impl Hir {
    /// Hand out the next place identity.
    ///
    /// Called wherever a `HirPlace` is built — including by instantiation,
    /// which is the whole point: a copied place must not share the
    /// original's identity even though it shares its span (#13).
    pub fn new_place(&mut self) -> PlaceId {
        let id = PlaceId::from_index(self.places as usize);
        self.places += 1;
        id
    }

    pub fn new() -> Self {
        Hir {
            defs: Arena::new(),
            locals: Arena::new(),
            exprs: Arena::new(),
            blocks: Arena::new(),
            view: None,
            prelude_defs: 0,
            prelude_exprs: 0,
            prelude_locals: 0,
            places: 0,
            routes: None,
        }
    }

    /// Whether this definition came from the prelude rather than from the
    /// file being compiled.
    pub fn is_prelude_def(&self, id: DefId) -> bool {
        id.index() < self.prelude_defs
    }

    /// Whether this expression came from the prelude.
    pub fn is_prelude_expr(&self, id: ExprId) -> bool {
        id.index() < self.prelude_exprs
    }

    /// Whether this binder came from the prelude.
    pub fn is_prelude_local(&self, id: LocalId) -> bool {
        id.index() < self.prelude_locals
    }

    /// Every definition the file being compiled declared, in source order.
    pub fn user_defs(&self) -> impl Iterator<Item = (DefId, &Def)> {
        self.defs.iter().filter(|(id, _)| !self.is_prelude_def(*id))
    }
}

impl Default for Hir {
    fn default() -> Self {
        Hir::new()
    }
}

/// A top-level declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Def {
    pub name: String,
    pub span: Span,
    pub kind: DefKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DefKind {
    Signal(Signal),
    Function(Function),
    View(View),
    Record(Record),
    Choice(Choice),
    Component(Component),
    Foreign(Foreign),
    Release(Release),
}

/// A `component` declaration (spec §14D.1).
///
/// The body is kept as written, never as instantiated. Each call site gets
/// its own copy, because a component's own `state` is per instance and its
/// parameters carry the caller's placements — so the graph the later passes
/// traverse is the *inlined* one (§14D.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub params: Vec<LocalId>,
    /// The binder for the nodes nested under this component at its call
    /// site, if it declared one.
    pub children: Option<LocalId>,
    /// The component's own state, in declaration order. Every one is
    /// `client`-placed: §14D.1 admits no other, because `server` state is
    /// per invocation and `durable` state is shared, so neither has a
    /// per-instance meaning.
    pub states: Vec<LocalSignal>,
    pub body: Vec<HirNode>,
}

/// A signal whose lifetime is one component instance rather than the
/// program.
///
/// It is a `Local` rather than a `Def` on purpose: a `Def` is emitted once
/// at module scope, and a component inside an `each` needs one signal per
/// row. Binding it as a local puts the declaration inside whichever region
/// closure the instance lands in, which is exactly per-instance.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalSignal {
    pub local: LocalId,
    pub placement: zdc_ast::Placement,
    pub ty: zdc_ast::TypeExpr,
    pub is_source: bool,
    /// The same clause a top-level [`Signal`] may carry, and the position
    /// where it earns its keep: a component instance is disposed when its
    /// row or its `when` arm goes away, so a clock declared here is torn
    /// down with it rather than ticking on into a page that no longer
    /// shows it.
    /// `every "90ms" starting v to <next>` — the fold a stepping clock
    /// performs, and `None` for every other signal including a plain
    /// clock.
    ///
    /// Held beside [`Signal::clock`] rather than inside it because the
    /// clock is a `Copy` description of a schedule and this is an
    /// expression in the module's arena. The pair is what says "this cell
    /// is written by the browser's scheduler, and here is what it is
    /// written *to*"; `clock` alone still means the elapsed-time reading
    /// it has always meant.
    ///
    /// **The step may read the signal it belongs to.** That is the one
    /// cycle the dependency graph permits, and it is the same one a
    /// `fold`'s accumulator is: the read takes the previous value, the
    /// write follows it, and nothing can observe the interval between.
    pub step: Option<ExprId>,
    pub clock: Option<zdc_ast::Clock>,
    pub init: ExprId,
    pub span: Span,
}

/// Where a `foreign`'s module specifier actually resolves (#238).
///
/// The specifier is what the program wrote and it is what the emitted
/// `import` says; this is what has to be true of the world for that import
/// to find anything. The distinction only exists because the two answers
/// are reached in different places — the browser resolves a URL and a path
/// by itself, and it resolves a bare name only from an import map the
/// document carries.
///
/// It is settled at resolution rather than at emission on purpose. The
/// alternative was to let the emitter read the project's mapping, which
/// meant a caller that built one without it emitted a bundle whose first
/// import failed — precisely the "compiles and cannot load" outcome this
/// exists to make impossible. Carried on the definition, there is one
/// answer and every emitter reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleTarget {
    /// The specifier resolves on its own: a relative or absolute path, an
    /// `http:`/`https:` URL, or the language's own `zd:` layer.
    AsWritten,
    /// A bare specifier, and the target the project's `[packages]` table
    /// mapped it to. The import still says the bare name — that is what
    /// the import map is for, and what lets several imports of one package
    /// be one module in the browser — except on the server, where there is
    /// no document to carry a map and the target is substituted directly.
    Mapped(String),
}

/// A `foreign` declaration: a platform function with no ZDeceptron body
/// (§14E, §17.4.2).
///
/// §14F.2 says the standard library is written in ZDeceptron and that
/// failing to write a piece of it "is a finding about the language, not a
/// reason to reach for the FFI". §17.4.10 records which pieces those are:
/// building a `Text` out of nothing, constructing a collection, f64
/// formatting, Unicode case tables, and the clock. Every other prelude
/// operation is an ordinary `Function` above these.
#[derive(Debug, Clone, PartialEq)]
pub struct Foreign {
    pub site: zdc_ast::ForeignSite,
    /// Where the symbol lives: a module this bundle imports, or the
    /// call's first argument — called, for a method, or read, for a
    /// property.
    pub source: zdc_ast::ForeignSource,
    /// What the imported module resolves to, decided once at resolution
    /// (#238).
    ///
    /// `None` exactly when there is no module to resolve — a method and a
    /// property both come with the receiver and import nothing. That is an `Option` rather
    /// than a defaulted [`ModuleTarget::AsWritten`] because the two are
    /// different facts: "the specifier resolves on its own" is an answer
    /// about a specifier, and a method has none to answer about.
    pub target: Option<ModuleTarget>,
    /// The symbol — an export name under `Import`, a method name under
    /// `Receiver`. Validated at parse time, and the type is what carries
    /// that refusal across the lowering: a `Foreign` holding a name that
    /// is not a JavaScript identifier does not exist to be emitted, in
    /// either position.
    pub export: zdc_ast::ExportName,
    pub form: zdc_ast::CallForm,
    pub params: Vec<LocalId>,
    /// The asserted parameter types, positionally matching `params`.
    pub param_types: Vec<zdc_ast::TypeExpr>,
    /// Which parameters were declared `takes p is trusted T` — obligation
    /// site A2, positionally matching `params`.
    pub trusted_params: Vec<bool>,
    /// What the `gives` line claims about the result (§21.9).
    ///
    /// `gives pure T` is grant `G-FGN-P` and `gives trusted T` is
    /// `G-FGN-T`. **`site` is not consulted by either**, and that
    /// separation is §21.8's repair: a placement answers which bundles a
    /// library may be linked into, and answering it can never establish
    /// that a result is a function of the arguments.
    ///
    /// A `gives view` foreign hands back no value to make a claim about,
    /// so the parser refuses a modifier on one and this stays
    /// [`zdc_ast::ForeignGrant::Opaque`] there.
    pub result_grant: zdc_ast::ForeignGrant,
    /// What the foreign hands back: a DOM node it owns, or a value of an
    /// asserted type.
    pub result: zdc_ast::ForeignResult,
}

impl Foreign {
    /// The module this is imported from, or `None` for a method, a
    /// property read or a property write, none of which imports anything.
    pub fn module(&self) -> Option<&str> {
        match &self.source {
            zdc_ast::ForeignSource::Import { module, .. } => Some(module),
            zdc_ast::ForeignSource::Receiver { .. }
            | zdc_ast::ForeignSource::Property { .. }
            | zdc_ast::ForeignSource::Write { .. } => None,
        }
    }

    /// Whether a call to this foreign is a method call on its first
    /// argument — `receiver.Export(…)`.
    pub fn is_method(&self) -> bool {
        matches!(self.source, zdc_ast::ForeignSource::Receiver { .. })
    }

    /// Whether a call to this foreign is a property read off its first
    /// argument — `receiver.Export`, with no call at all.
    pub fn is_property(&self) -> bool {
        matches!(self.source, zdc_ast::ForeignSource::Property { .. })
    }

    /// Whether a call to this foreign writes a property of its first
    /// argument — `receiver.Export = value`.
    pub fn writes_property(&self) -> bool {
        matches!(self.source, zdc_ast::ForeignSource::Write { .. })
    }

    /// Whether this names the language's own primitive layer rather than a
    /// package on the platform (§17.4.10).
    pub fn is_primitive(&self) -> bool {
        self.module()
            .is_some_and(|module| module.starts_with("zd:"))
    }

    /// Whether this foreign owns a DOM node rather than returning a value.
    ///
    /// The two forms differ in exactly this, which is why they are one
    /// declaration (§4.1) and one enum rather than two of each.
    pub fn owns_view(&self) -> bool {
        matches!(self.result, zdc_ast::ForeignResult::View)
    }

    /// Whether a call to this foreign **constructs** — `new Export(…)`
    /// rather than `Export(…)`.
    ///
    /// Read off the `gives` line and nowhere else, so a reader asking what
    /// a foreign hands back and a reader asking how it is applied read one
    /// clause and not two.
    pub fn constructs(&self) -> bool {
        matches!(self.result, zdc_ast::ForeignResult::New(_))
    }
}

/// A `record` declaration: a product type with named fields (§14B.1).
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// In declaration order. Codegen emits object literals in this order so
    /// every instance of a record shares one hidden class (§16.7 item 9).
    pub fields: Vec<Field>,
}

/// A `choice` declaration: a tagged union (§14B.1, §14G.1.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub variants: Vec<Variant>,
}

/// Why a cell refuses a write, when it does.
///
/// Two passes and the emitter all have to answer the same question about
/// the same two fields, and before this existed each answered it with its
/// own sentence — which is how `zdc check` and the language server came to
/// describe one refusal in two ways. The question is asked here once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NotWritable {
    /// `from` — the compiler recomputes it.
    Derived,
    /// `every` / `after` — the clock writes it.
    Clock(zdc_ast::Clock),
}

impl NotWritable {
    /// Whether a cell declared this way refuses writes, and why.
    ///
    /// **A stepping clock's cell accepts writes**, which is why `stepping`
    /// is a parameter rather than inferred from `clock`. A plain clock's
    /// value is the *compiler's* — elapsed milliseconds — and writing it
    /// would be writing over an answer the program did not compute and
    /// cannot mean anything by. A stepping clock's value is the program's,
    /// written by its own `starting` and its own step, and the scheduler
    /// is one writer among several rather than the only one: a board that
    /// advances on a timer still has to accept `press g to stamp a
    /// pattern`, and refusing that would make the construct useless for
    /// every game it exists to serve.
    pub fn of(
        is_source: bool,
        clock: Option<zdc_ast::Clock>,
        stepping: bool,
    ) -> Option<NotWritable> {
        match (is_source, clock) {
            (_, Some(_)) if stepping => None,
            (_, Some(clock)) => Some(NotWritable::Clock(clock)),
            (false, None) => Some(NotWritable::Derived),
            (true, None) => None,
        }
    }

    /// The sentence a diagnostic says about `name`.
    ///
    /// It names the clause the program wrote, because "nothing can write
    /// to it" without that is a rule the reader has to go and look up.
    pub fn refusal(self, name: &str) -> String {
        match self {
            NotWritable::Derived => format!(
                "`{name}` is derived with `from`, so nothing can write to it. It is recomputed \
                 from what it reads."
            ),
            NotWritable::Clock(clock) => format!(
                "`{name}` is written by the clock — `{}` — so nothing in the program can write \
                 to it. Derive what you need from it with `from`.",
                clock.written()
            ),
        }
    }
}

/// One variant of a `choice`, with its named fields in declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    /// What a person is shown where this variant is read rather than
    /// matched. `None` where the declaration gave none, and the name
    /// stands in — kept as an `Option` rather than defaulted here so that
    /// "the program said nothing" and "the program said the same as the
    /// name" remain different facts to anything that reads this later.
    pub label: Option<String>,
    pub fields: Vec<Field>,
    pub span: Span,
}

/// One `name is type` field of a record or of a variant's payload.
///
/// The type is not resolved here, for the same reason a signal's is not:
/// this pass resolves names to definitions, and a type name has a meaning
/// to check only once there is a checker.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: zdc_ast::TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub secret: bool,
    /// `trusted state` — spec §18.1.1.
    ///
    /// Two jobs in one word, and §18.1.6 limit 2 says so plainly: it is the
    /// **grant** that makes a read of this signal Trusted (`G-SIG` clause 1,
    /// §21.7.3), *and* it is the **obligation** that makes every write to
    /// this place (A3) and every index into it (A1) a checked site.
    pub trusted: bool,
    pub placement: zdc_ast::Placement,
    /// Types are not resolved by this pass; they are checked by the next
    /// one, which is where a type name has a meaning to check against.
    pub ty: zdc_ast::TypeExpr,
    /// `true` for `starting` (a mutable source), `false` for `from` (a
    /// derived value). Spec §4.5.
    ///
    /// A clock signal is `false` here too — nothing in the program may
    /// write it — and [`Signal::clock`] is what tells the two apart where
    /// the difference shows: in a diagnostic's wording and in the emitted
    /// declaration.
    pub is_source: bool,
    /// `every "250ms"`, `every frame`, `after "2s"` — the clock that writes
    /// this cell, if one does (#19).
    ///
    /// **Present here rather than as a third `DefKind`** because a clock
    /// signal is a signal in every way that the rest of the compiler cares
    /// about: it has a placement, a type, a read label and a set of
    /// readers. What differs is one line of emission and who does the
    /// writing.
    /// The fold a stepping clock performs — see `LocalSignal::step`,
    /// which carries the same field for a component's own cell.
    pub step: Option<ExprId>,
    pub clock: Option<zdc_ast::Clock>,
    /// `every "1h"` and its block — the deployment's scheduler writes this
    /// cell, and the block is what runs when it does (§14G.4).
    ///
    /// Beside [`Signal::clock`] rather than sharing it, and the two are
    /// mutually exclusive: a clock is a *source with no code*, which is
    /// what lets a browser timer stay out of control flow entirely, and a
    /// schedule is a *root with a body*, because a `server` cell has no
    /// reader unless the job itself is one. Folding them into one optional
    /// field would put a block behind an `Option` that half the compiler
    /// reads as "no block, ever".
    pub schedule: Option<Schedule>,
    /// The value the cell holds before the clock has written it — `0` for
    /// an elapsed-milliseconds signal, `no` for a delay.
    ///
    /// A real expression, not a special case: it is what the type checker
    /// checks the annotation against and what the integrity pass reads for
    /// G-SIG clause 2, so a clock signal travels every pass that a
    /// `starting` signal does.
    pub init: ExprId,
    /// §14C.3b: the path this value is written to at build time, if any.
    ///
    /// Carried on the signal rather than on a declaration of its own,
    /// because it *is* a property of the state: `rss.xml` is the value of
    /// `feed`, so there is nothing to keep in sync with anything.
    pub emits: Option<zdc_ast::Emitted>,
    /// The span of the `expect` clause this signal came from, when it came
    /// from a `test` declaration rather than a `state` one — issue #169.
    ///
    /// # Why a `test` is a signal at all
    ///
    /// A test is a value that is computed once, at build time, from
    /// nothing but the program: that is the definition of `static`
    /// placement (§14C.3b), word for word. Lowering it to anything else
    /// would mean a second answer to every question the passes already
    /// answer for a `static` signal — where does it run, what may it read,
    /// what is its type, which functions does it pull into which root —
    /// and a second answer is a second thing to get wrong.
    ///
    /// So a test *is* a `static Truth`, and this field is the one bit that
    /// distinguishes it. It carries a span rather than a `bool` because
    /// every use of it needs the span: the split needs it to classify the
    /// member, and the runner needs it to point a broken claim's caret at
    /// the line the reader wrote. `Option<Span>` is therefore not a
    /// boolean in disguise — the `Some` arm carries the only thing anyone
    /// asks for after the question is answered.
    ///
    /// The claim itself is the definition's `name`: a test is registered
    /// in no scope (see `zdc-resolve`'s `collect`), so its name is free to
    /// be the sentence the report prints rather than an identifier.
    pub expectation: Option<zdc_lexer::Span>,
}

/// A job the deployment runs on a schedule — §14G.4's `every` on a
/// `server` declaration.
///
/// # Why the body hangs off the signal instead of being its own `DefKind`
///
/// It is the same argument [`Signal::clock`] makes and it is stronger
/// here. §14G.4 revision 1 withdrew the top-level `on IDENT` handler
/// because its identifier resolved in the union of two disjoint
/// namespaces — DOM event names and signal names — with nothing lexical
/// selecting between them, and two reviewers exhibited the collision
/// independently. Attaching the block to the declaration removes the
/// production and therefore the collision, and it makes three further
/// rules structural rather than stated: one handler per trigger, because
/// a declaration has one block; a trigger with no handler is impossible,
/// because the block is not optional; and the cycle check has nothing to
/// inspect, because no handler is attached to a *change* and so the graph
/// cannot self-excite.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub cadence: zdc_ast::Cadence,
    pub body: BlockId,
    /// The `every "…"` clause, without the block. What a diagnostic about
    /// the cadence points at.
    pub span: zdc_lexer::Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// How every call to this function must be written (§17.4.2). A
    /// `with` function called with `of`, or the reverse, is an error that
    /// names the one valid form.
    pub form: zdc_ast::CallForm,
    pub params: Vec<LocalId>,
    pub body: BlockId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// The document's metadata, already reduced to the literals it must
    /// be. It is written into `index.html` at build time, so it cannot
    /// read a signal: there is nothing at run time to write it into.
    pub metadata: Metadata,
    pub nodes: Vec<HirNode>,
}

/// What `view title is "…", description is "…", language is "…"` said.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Metadata {
    /// The `<title>`. `None` means the source file's stem is used.
    pub title: Option<String>,
    /// The `<meta name="description">`, omitted when absent.
    pub description: Option<String>,
    /// `<html lang>`, which defaults to `en`.
    pub language: Option<String>,
}

/// The named arguments a `view` accepts, so the diagnostic and the
/// reader agree on the list.
pub const VIEW_METADATA: &[&str] = &["title", "description", "language"];

/// A `release` declaration — spec §19.1, §19.10.2.
///
/// The one construct that produces a Public result from Secret inputs.
/// Structurally it is a function with three extra clauses, and it is
/// deliberately *not* a `Function`: every rule that quantifies over release
/// declarations — REL-ARG, REL-CLOSED, REL-PURE, REL-PLACE′ — needs the set
/// to be enumerable by the parser, which is what makes the audit complete
/// by grammar rather than by diligence (§19.5).
///
/// **No robustness property is claimed for any of it.** Three adversarial
/// passes broke the claim in turn (§19.9, §19.11, §21.8); the rules are
/// worth having as review aids and are built on those terms (§21.8.8
/// option 2).
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub params: Vec<LocalId>,
    /// The declared bandwidth per evaluation (§19.2 rule 4).
    pub gives: zdc_ast::TypeExpr,
    /// `endorsed(f)` in REL-ARG, positionally matching `params`.
    ///
    /// Site-local and result-transparent: it discharges REL-ARG at this
    /// release's call sites and raises nothing inside the body, because
    /// raising the label inside would make the release a universal
    /// integrity launderer (§19.10.3(a)).
    pub endorsed: Vec<bool>,
    /// `limit N per visitor`, if written.
    ///
    /// **Not a disclosure bound.** Per declaration and per anonymous
    /// session: `k` declarations give `kN`, a cookie clear resets it, and
    /// nothing enforces it until `DurableStore` exists (§21.8.7, R3).
    pub limit: Option<ReleaseBudget>,
    pub body: BlockId,
}

/// `limit N per visitor` — see [`Release::limit`] for what it does not do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseBudget {
    pub count: u32,
    pub span: Span,
}

/// A binding introduced inside a body: a parameter, a loop variable, or
/// one of a pattern's binders.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExprKind {
    /// `value if condition otherwise other`.
    ///
    /// Three expressions and no block: this is a *value*, so there is
    /// nothing here to sequence and nothing to fall off the end of. The
    /// statement `if` keeps its own lowering, and neither is written in
    /// terms of the other — a statement may `give` from either arm or
    /// from neither, and an expression must produce one value from
    /// exactly one of two.
    Conditional {
        condition: ExprId,
        value: ExprId,
        otherwise: ExprId,
    },
    Number(f64),
    Text(String),
    Truth(bool),
    Empty,
    /// `[a, b]` — spec §14B.4.
    List(Vec<ExprId>),
    /// `["a" to 1]` — spec §14B.4, in written order.
    Map(Vec<(ExprId, ExprId)>),
    /// A resolved reference. The string is gone.
    Ref(Res),
    Call {
        callee: Res,
        args: Vec<HirArg>,
    },
    /// `length of items` — a call in the `of` form (§14F.1, §17.4.2).
    ///
    /// Kept apart from `Call` because the two are not interchangeable: the
    /// declaration decides which spelling a callable answers to, and
    /// collapsing them here would lose the only thing that distinguishes
    /// them by the time the checker could report it.
    OfCall {
        callee: Res,
        operand: ExprId,
    },
    /// `length of` and `text of` — the two members of §17.4.3's closed
    /// dispatched set that no ZDeceptron body can define, whichever type
    /// they are applied to.
    ///
    /// Which primitive this means is chosen by the head constructor of its
    /// operand, so the checker settles it and records the answer; codegen
    /// reads that verdict rather than guessing one.
    Operator {
        op: OperatorName,
        operand: ExprId,
    },
    Environment(String),
    /// `address` — the URL this document was served at, as
    /// `Option of <route>` (spec §14G.2).
    Address,
    /// `media "(prefers-color-scheme: dark)"` — whether the browser
    /// matches a CSS media query, as a `Truth`.
    ///
    /// It carries the query verbatim, as [`HirExprKind::Environment`]
    /// carries its key: what a browser will do with the string is not a
    /// question this compiler can answer, and there is no closed set to
    /// check it against — CSS grows media features without asking.
    ///
    /// **It is a signal read, not a call.** The value changes when the
    /// visitor changes their system theme, resizes, or turns animation
    /// off, and the emitter therefore lowers it to a read of a cell the
    /// runtime keeps subscribed. That is the whole reason it is a language
    /// construct rather than a `foreign`: a `foreign` would be called
    /// once, and reading `matchMedia(q).matches` once is the exact bug the
    /// survey of the target site found in six of its eight call sites.
    Media(String),
    /// `scroll` — how far down the document the reader is, 0 to 100.
    ///
    /// Carries nothing, because there is one document and one answer.
    Scroll,
    /// `build read path` — a capability the compiler itself supplies.
    ///
    /// The name has already been checked against [`BuildCapability`]'s
    /// closed set by name resolution, so nothing downstream carries a
    /// string it has to re-validate.
    Build {
        capability: BuildCapability,
        argument: ExprId,
    },
    Unary {
        op: zdc_ast::UnaryOp,
        operand: ExprId,
    },
    Binary {
        op: zdc_ast::BinOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    /// A field name stays a string: which record it selects from is not
    /// known until types are.
    Field {
        base: ExprId,
        name: String,
    },
    Index {
        base: ExprId,
        index: ExprId,
    },
    /// `append item to list` — the list construction form.
    ///
    /// The one operation that makes a list *longer*. `rest of` makes one
    /// shorter and, before this, nothing made one longer, so no function
    /// could hand back a collection it had not been given — which is what
    /// kept `split`, `reverse` and `values` in the primitive layer.
    Append {
        item: ExprId,
        list: ExprId,
    },
    /// `set key to value in table`: the map construction form.
    ///
    /// The one operation that makes a map *bigger*. Nothing made one at
    /// all before it: `map.zd`'s three primitives take a map apart, so no
    /// prelude function could hand one back, which is why `remove`,
    /// `merge`, `mapValues` and building a map from a list were all
    /// unwritable at once.
    Insert {
        key: ExprId,
        value: ExprId,
        table: ExprId,
    },
    /// A `request` declaration's initialiser — the outbound request (#19).
    ///
    /// The one expression in the language that leaves the machine it is
    /// evaluated on. It is an expression rather than a statement because
    /// §5's `Remote of T` is what it produces and a `Remote` is a value:
    /// it recomputes when [`HirExprKind::Outbound::args`] change, exactly
    /// as `$remote` does for a generated endpoint, and the browser cannot
    /// reach the body without eliminating the variant.
    ///
    /// **It has no syntax of its own.** `zdc-resolve` builds exactly one
    /// of these, from a [`zdc_ast::RequestDecl`], and puts it in that
    /// declaration's signal initialiser. So there is one per declaration,
    /// at the top level of a file, which is what makes "read the program
    /// to see what it talks to" a true sentence.
    Outbound {
        /// The destination, already checked by [`crate::destination`]. A
        /// `String` and not an `ExprId`: there is no expression, and the
        /// type is what carries that across every later pass.
        destination: String,
        /// The query parameters, in source order. Each is
        /// [`HirArg::Named`] — a query parameter has a name in the URL.
        ///
        /// **These are the sink's producing site.** They are appended to
        /// the destination as a query string, so an argument is part of
        /// the URL the browser sends, and §14G.1.3(c)'s sink 7 rules on
        /// each one separately.
        args: Vec<HirArg>,
    },
    /// `map each x in maybe to x * 2` — transform the payload of an
    /// `Option` or a `Remote`, leaving the other arms alone (#103, #104).
    ///
    /// **The one expression that binds a local**, and every pass that
    /// collects binders had to be told which kind this is. Before it,
    /// every binder in the language belonged to a statement or a
    /// declaration, so two assumptions had grown up unstated and both are
    /// now written down where they are relied on:
    ///
    /// * A walk over statements alone does not see this binder.
    ///   `zdc-codegen`'s `block_binders` and `declaration_block_binders`
    ///   descend into expressions because of it.
    /// * A binder in a *view* is not automatically reactive. This one is
    ///   the parameter of an arrow the emitter writes and holds a plain
    ///   value, unlike an `each` binder or a `when` pattern binding, which
    ///   the runtime hands over as getters. `node_binders` therefore stays
    ///   out of expressions on purpose: collecting this one there emits
    ///   `x()` against a number, which renders as a crash at first paint.
    ///
    /// `zdc-resolve`'s `Instantiator::expr` is the third: it is the only
    /// arm of that walk that calls `rebind`, without which two instances
    /// of one component would share this binder.
    MapInside {
        var: LocalId,
        source: ExprId,
        to: ExprId,
    },
}

/// A built-in unary operator written with `of`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorName {
    /// `length of` — over `Text`, `List of T`, and `Map of K to V`.
    Length,
    /// `text of` — over every base type.
    TextOf,
}

impl OperatorName {
    pub fn from_name(name: &str) -> Option<OperatorName> {
        Some(match name {
            "length" => OperatorName::Length,
            "text" => OperatorName::TextOf,
            _ => return None,
        })
    }

    /// How it reads in a diagnostic, as the program wrote it.
    pub fn describe(self) -> &'static str {
        match self {
            OperatorName::Length => "length of",
            OperatorName::TextOf => "text of",
        }
    }
}

/// The capabilities a build may ask the compiler for — the closed set.
///
/// **Why a closed set, and not a module loader.** A runtime `foreign`
/// calls into a real host: a browser, or a serverless runtime, which
/// genuinely has npm and a DOM. A build-time call has no host — the
/// compiler *is* the host — so the honest construct is not "import a
/// module" but "ask the compiler for a capability". Everything here is
/// pure Rust under `#![forbid(unsafe_code)]`, every path is resolved
/// against the project directory before it is opened, and every answer is
/// deterministic, which is what §17.4.7 asks of a build.
///
/// The cost is stated rather than argued away: **this set grows only with
/// compiler releases.** What bounds the cost is that growing it spends no
/// keyword — the capability name is an identifier in the `build`
/// production, so a tenth capability costs a match arm and nothing from
/// §14G.7.7's budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuildCapability {
    /// `build read path` — one file's contents, as `Text`.
    Read,
    /// `build list directory` — the files directly inside a directory, as
    /// `List of Text`, each relative to the project directory and **sorted
    /// by byte order**, because a filesystem's own order is not a fact
    /// about the program.
    List,
    /// `build markdown source` — CommonMark rendered to HTML, as `Text`.
    Markdown,
    /// `build parts source` — one document split into a `List of Part`
    /// (issue #305).
    ///
    /// The capability `Markdown` could not be. `Prose` renders one
    /// `Markup` and has no children, because interleaving parsed nodes
    /// with templated ones would make the sibling offsets every binding is
    /// scheduled against depend on how many nodes a *file* parsed into. So
    /// a document that wants an interactive chart in the middle of it
    /// cannot be one node, and this is the capability that makes it a
    /// list: prose runs and named widgets, alternating, each its own node
    /// under an ordinary `each`.
    ///
    /// It renders the prose with the same pass `Markdown` uses, so
    /// everything that is true of a rendered post is true of a rendered
    /// part.
    Parts,
}

/// The record `build parts` hands back one of per part — issue #305.
///
/// Declared in the prelude (`prelude/parts.zd`) rather than built in, so
/// that field access, construction and `each` need no rule for it. Named
/// here because the checker gives `build parts` this type and the
/// evaluator builds values of it, and a name those two disagreed about
/// would be a `List of Part` no program could read.
pub const PART_RECORD: &str = "Part";

/// `Part`'s fields, in declaration order.
///
/// One list, read by the checker and by the sandbox that builds the
/// values, for the reason `Type::PAIR_FIELDS` is one list: two spellings
/// of a record's shape is a record two passes disagree about.
pub const PART_FIELDS: [&str; 3] = ["markup", "widget", "argument"];

/// The `choice` a program declares to say which widgets a document may
/// name — issue #305.
///
/// **This is the closed set, and the program owns it.** A component cannot
/// be resolved from a file's text: components are resolved statically and
/// a name out of a `.md` is not a name the compiler saw. So the program
/// declares what a post may ask for, dispatches on the name with a `when`
/// over this choice, and a document naming anything else is a **failed
/// build** rather than a blank space — which is a stronger bargain than
/// MDX makes, where an `import` inside a content file can reach anything
/// on disk.
///
/// Located by name, which is the one thing this design needed that the
/// language had no spelling for. The alternative was a keyword, and
/// §14G.7.7's budget is not worth spending on a declaration that is
/// already a `choice` in every respect but this one.
pub const WIDGET_CHOICE: &str = "Widget";

impl BuildCapability {
    /// The closed set, in the order a diagnostic should list it.
    pub const ALL: [BuildCapability; 4] = [
        BuildCapability::Read,
        BuildCapability::List,
        BuildCapability::Markdown,
        BuildCapability::Parts,
    ];

    /// The one spelling of this capability's name.
    pub fn name(self) -> &'static str {
        match self {
            BuildCapability::Read => "read",
            BuildCapability::List => "list",
            BuildCapability::Markdown => "markdown",
            BuildCapability::Parts => "parts",
        }
    }

    /// The capability that name selects, or `None` if the set has none.
    pub fn from_name(name: &str) -> Option<BuildCapability> {
        BuildCapability::ALL
            .into_iter()
            .find(|capability| capability.name() == name)
    }

    /// What one costs, for a diagnostic that has to say why it refused.
    pub fn describe(self) -> &'static str {
        match self {
            BuildCapability::Read => "reads a file from the project directory",
            BuildCapability::List => "lists the files in a directory of the project",
            BuildCapability::Markdown => "renders CommonMark to HTML",
            BuildCapability::Parts => "splits a document into prose runs and the widgets it names",
        }
    }
}

/// The `of`-operator names no user declaration may take, because a
/// program writing `length of` must always mean the same thing (§4.1).
pub const BUILTIN_OF_OPERATORS: &[&str] = &["length", "text"];

#[derive(Debug, Clone, PartialEq)]
pub enum HirArg {
    Positional(ExprId),
    Named { name: String, value: ExprId },
}

/// The one element whose leading argument is a URL: `Link`.
pub const DESTINATION_ELEMENT: &str = "Link";

/// The attribute a `Link`'s leading argument *is*, named here rather than
/// left implicit in the slot.
///
/// # Why the destination is a named argument in the HIR
///
/// A `Link`'s destination is written first — `Link "https://example.com"`,
/// `Link Home` — and a leading argument is otherwise lowered by the slot,
/// which is a position rather than a name. Every pass that ranges over
/// *URL-bearing attributes* ranges over attribute **names**: it asks
/// whether an argument is `href`, `src`, `srcset` and so on. A destination
/// carried only by its position would be invisible to every one of them —
/// and it would be invisible for the commonest way there is to write a
/// link, so the rule would look enforced and would not be.
///
/// So the destination is not a nameless slot in the HIR. `zdc-resolve`
/// puts it under this name the moment it lowers the element, and from
/// there it is an ordinary [`HirArg::Named`] carrying the attribute it
/// becomes. A name-keyed URL rule sees it without knowing that `Link`
/// exists, and codegen sends it down the same path a named URL argument
/// takes. The source syntax is unchanged and stays single: writing
/// `href is …` on a `Link` is a resolve error naming the one phrasing.
pub const DESTINATION_ARGUMENT: &str = "href";

/// The destination argument of an element, if it has one.
///
/// The counterpart of [`destination_as_href`]: every pass that wants
/// *where this link goes* asks here rather than reaching for the leading
/// positional argument, which no longer holds it.
pub fn destination_of(element: &HirElement) -> Option<ExprId> {
    if element.name != DESTINATION_ELEMENT {
        return None;
    }
    element.args.iter().find_map(|arg| match arg {
        HirArg::Named { name, value } if name == DESTINATION_ARGUMENT => Some(*value),
        HirArg::Named { .. } | HirArg::Positional(_) => None,
    })
}

/// Rewrite a `Link`'s leading destination into the `href` it becomes.
///
/// Only the first positional argument is rewritten. A second one is not a
/// destination and is left where it is, so the type checker still reports
/// it as the extra leading value it is rather than as a missing `href`.
pub fn destination_as_href(element: &str, mut args: Vec<HirArg>) -> Vec<HirArg> {
    if element != DESTINATION_ELEMENT {
        return args;
    }
    for arg in &mut args {
        if let HirArg::Positional(value) = *arg {
            *arg = HirArg::Named {
                name: DESTINATION_ARGUMENT.to_string(),
                value,
            };
            break;
        }
    }
    args
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirBlock {
    pub stmts: Vec<HirStmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmt {
    Pipeline(HirPipeline),
    Mutation(HirMutation),
    Give(ExprId),
    When(HirWhen),
    Each(HirEach),
    If(HirIf),
    /// `with total is 0` — spec §17.4.10's local binding.
    Bind(HirBind),
    /// `do render with r is gl` — one call, run for its effect.
    ///
    /// The expression is whole, so every pass reaches the call through the
    /// ordinary expression walk. Nothing downstream needs a rule for a
    /// second kind of call site, which is what keeps the information-flow
    /// walk's coverage a property of one function rather than of a list.
    Do(HirDo),
}

/// One `do` statement: the call, and where it was written.
#[derive(Debug, Clone, PartialEq)]
pub struct HirDo {
    pub call: ExprId,
    pub span: Span,
}

/// One `with` statement's run of bindings, in written order.
///
/// Not a scope of its own: a binding is in scope from the statement after
/// it to the end of the block it was written in, which is the block's
/// scope and no new one. This is the same decision §14D's `HirScope`
/// records for a component instance — a construct that binds names
/// without being a region boundary — and it is why nothing downstream of
/// resolution needs a rule for bindings at all.
#[derive(Debug, Clone, PartialEq)]
pub struct HirBind {
    pub bindings: Vec<HirBinding>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirBinding {
    pub local: LocalId,
    pub value: ExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirPipeline {
    From(ExprId),
    Keep {
        var: LocalId,
        cond: ExprId,
    },
    Sort {
        var: LocalId,
        key: ExprId,
    },
    MapEach {
        var: LocalId,
        to: ExprId,
    },
    /// `fold each n into total starting 0 to total + n` (#33).
    ///
    /// `starting` is evaluated once, in the scope outside the clause;
    /// `step` once per element, with both binders in scope. The clause is
    /// terminal — see `zdc_ast::PipelineClause::Fold`.
    Fold {
        item: LocalId,
        total: LocalId,
        starting: ExprId,
        step: ExprId,
    },
    TakeFirst(ExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirMutation {
    Set { place: HirPlace, value: ExprId },
    Add { value: ExprId, place: HirPlace },
    Subtract { value: ExprId, place: HirPlace },
    Append { value: ExprId, place: HirPlace },
    Remove { value: ExprId, place: HirPlace },
}

impl HirMutation {
    pub fn place(&self) -> &HirPlace {
        match self {
            HirMutation::Set { place, .. }
            | HirMutation::Add { place, .. }
            | HirMutation::Subtract { place, .. }
            | HirMutation::Append { place, .. }
            | HirMutation::Remove { place, .. } => place,
        }
    }

    pub fn value(&self) -> ExprId {
        match self {
            HirMutation::Set { value, .. }
            | HirMutation::Add { value, .. }
            | HirMutation::Subtract { value, .. }
            | HirMutation::Append { value, .. }
            | HirMutation::Remove { value, .. } => *value,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirPlace {
    /// Identity, allocated fresh for every place — including for each copy
    /// instantiation makes of a component's body.
    ///
    /// `span` cannot serve: two instances of one component carry the same
    /// spans, so a map keyed on one conflates their writes. `base` cannot
    /// either, because a component writing a top-level signal has the same
    /// `DefId` in every instance (#13).
    pub id: PlaceId,
    pub base: Res,
    pub path: Vec<HirPathSeg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirPathSeg {
    Field(String),
    Index(ExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirWhen {
    pub scrutinee: ExprId,
    pub arms: Vec<HirArm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirArm {
    /// The variant matched. Which choice it belongs to is a question for
    /// the type checker, so the name is still a string here.
    pub pattern_name: String,
    /// One binder per named field of the matched variant, in declaration
    /// order (spec §14G.1.2). Empty for a payload-free variant such as
    /// `Loading`.
    pub bindings: Vec<LocalId>,
    pub body: HirArmBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirArmBody {
    Show(ExprId),
    Block(BlockId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirEach {
    pub var: LocalId,
    pub iter: ExprId,
    pub body: BlockId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirIf {
    pub cond: ExprId,
    pub then: BlockId,
    pub otherwise: Option<BlockId>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirNode {
    Element(HirElement),
    Each(HirEachNode),
    When(HirWhenNode),
    If(HirIfNode),
    Handler(HirHandler),
    /// `children`, before instantiation replaces it with the nodes nested
    /// under the call site. No `Children` survives into a `view`.
    Children(Span),
    /// One component instance: its own state, and the body that reads it.
    ///
    /// Produced by instantiation, never by the parser. It is not a region
    /// boundary — the locals are declared in whatever region the instance
    /// lands in, so an instance inside an `each` row gets its state inside
    /// that row's closure.
    Scope(HirScope),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirScope {
    pub locals: Vec<LocalSignal>,
    pub body: Vec<HirNode>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirIfNode {
    pub cond: ExprId,
    pub then: Vec<HirNode>,
    pub otherwise: Option<Vec<HirNode>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirElement {
    pub name: String,
    pub res: Res,
    pub args: Vec<HirArg>,
    pub children: Vec<HirNode>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirEachNode {
    pub var: LocalId,
    pub iter: ExprId,
    pub body: Vec<HirNode>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirWhenNode {
    pub scrutinee: ExprId,
    pub arms: Vec<HirNodeArm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirNodeArm {
    pub pattern_name: String,
    /// One binder per named field, in declaration order (spec §14G.1.2).
    pub bindings: Vec<LocalId>,
    pub body: HirNodeArmBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirNodeArmBody {
    Show(Box<HirElement>),
    Nodes(Vec<HirNode>),
}

/// What raises the event a handler runs on — see [`zdc_ast::HandlerTarget`].
///
/// A field on [`HirHandler`] rather than a second [`HirNode`] variant, and
/// that is the load-bearing choice. Twelve walks in five crates reach a
/// handler only to descend into `handler.body` — the flow pass, the
/// integrity pass, `sites_of`, the type checker, the router, the placement
/// scan, three codegen analyses. A new variant would have left every one of
/// them to be taught about a body it must not skip, and a walk that skips a
/// body **fails open**: the statements inside are simply never checked. A
/// field cannot be skipped, because there is nothing new to match on.
///
/// What genuinely differs is checked where it differs: which region may
/// carry it (`split.rs`), whether it may hang off an element (`view.rs`),
/// and what it emits.
#[derive(Debug, Clone, PartialEq)]
pub enum HandlerTarget {
    /// The element this handler is written under.
    Element,
    /// The document, for one named key — `on key "Escape"` (§16.3.7a).
    Document { key: String, key_span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirHandler {
    pub event: String,
    /// The binder of `on click with press`, if the handler asked for the
    /// event. A `Local` rather than anything new: it is a name bound over
    /// a body, which is what every other binder in the language is, so
    /// scoping, naming and emission all reuse the machinery that exists.
    ///
    /// Always `None` for a [`HandlerTarget::Document`]: that production has
    /// no `with` in it.
    pub payload: Option<LocalId>,
    pub target: HandlerTarget,
    /// Where the event name was written, for the diagnostic that has to
    /// name an event the language does not know.
    pub event_span: Span,
    pub body: BlockId,
    pub span: Span,
}
