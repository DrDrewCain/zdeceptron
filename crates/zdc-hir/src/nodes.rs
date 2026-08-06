//! The HIR node types.
//!
//! Every node carries the span of the source it came from: later passes
//! report their errors against HIR rather than AST, so a node without a
//! span is a diagnostic that cannot point anywhere.

use crate::ids::{Arena, ArenaId, BlockId, DefId, ExprId, LocalId};
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
    // links and media
    Link,
    Image,
    Figure,
    Caption,
    Canvas,
    // controls
    Button,
    Input,
    Checkbox,
    Label,
    Fieldset,
    Legend,
    Spinner,
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
    pub const ALL: [BuiltinElement; 48] = [
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
        BuiltinElement::Link,
        BuiltinElement::Image,
        BuiltinElement::Figure,
        BuiltinElement::Caption,
        BuiltinElement::Canvas,
        BuiltinElement::Button,
        BuiltinElement::Input,
        BuiltinElement::Checkbox,
        BuiltinElement::Label,
        BuiltinElement::Fieldset,
        BuiltinElement::Legend,
        BuiltinElement::Spinner,
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
        "Link",
        "Image",
        "Figure",
        "Caption",
        "Canvas",
        "Button",
        "Input",
        "Checkbox",
        "Label",
        "Fieldset",
        "Legend",
        "Spinner",
        "ErrorBar",
    ];

    /// Whether this element writes back into the signal bound to its first
    /// positional argument on every interaction (spec §14B.5).
    pub fn is_two_way(self) -> bool {
        matches!(self, BuiltinElement::Input | BuiltinElement::Checkbox)
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
    /// [`is_url_attribute`], which ranges over the attribute name on every
    /// element, because `named_argument` passes an unrecognised name
    /// through to the attribute of that name: `Text src is …` reaches the
    /// DOM whether or not `Text` was meant to have a `src`. The two are
    /// tied together by a test.
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
            | BuiltinElement::Figure
            | BuiltinElement::Caption
            | BuiltinElement::Canvas
            | BuiltinElement::Button
            | BuiltinElement::Input
            | BuiltinElement::Checkbox
            | BuiltinElement::Label
            | BuiltinElement::Fieldset
            | BuiltinElement::Legend
            | BuiltinElement::Spinner
            | BuiltinElement::ErrorBar => &[],
            BuiltinElement::Image => &["source"],
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
    pub init: ExprId,
    pub span: Span,
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
    pub module: String,
    /// The export within that module. Validated at parse time, and the
    /// type is what carries that refusal across the lowering: a `Foreign`
    /// holding an export that is not a JavaScript identifier does not
    /// exist to be emitted.
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
    /// Whether this names the language's own primitive layer rather than a
    /// package on the platform (§17.4.10).
    pub fn is_primitive(&self) -> bool {
        self.module.starts_with("zd:")
    }

    /// Whether this foreign owns a DOM node rather than returning a value.
    ///
    /// The two forms differ in exactly this, which is why they are one
    /// declaration (§4.1) and one enum rather than two of each.
    pub fn owns_view(&self) -> bool {
        matches!(self.result, zdc_ast::ForeignResult::View)
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

/// One variant of a `choice`, with its named fields in declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
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
    pub is_source: bool,
    pub init: ExprId,
    /// §14C.3b: the path this value is written to at build time, if any.
    ///
    /// Carried on the signal rather than on a declaration of its own,
    /// because it *is* a property of the state: `rss.xml` is the value of
    /// `feed`, so there is nothing to keep in sync with anything.
    pub emits: Option<zdc_ast::Emitted>,
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
}

impl BuildCapability {
    /// The closed set, in the order a diagnostic should list it.
    pub const ALL: [BuildCapability; 3] = [
        BuildCapability::Read,
        BuildCapability::List,
        BuildCapability::Markdown,
    ];

    /// The one spelling of this capability's name.
    pub fn name(self) -> &'static str {
        match self {
            BuildCapability::Read => "read",
            BuildCapability::List => "list",
            BuildCapability::Markdown => "markdown",
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
    Keep { var: LocalId, cond: ExprId },
    Sort { var: LocalId, key: ExprId },
    MapEach { var: LocalId, to: ExprId },
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

#[derive(Debug, Clone, PartialEq)]
pub struct HirHandler {
    pub event: String,
    /// The binder of `on click with press`, if the handler asked for the
    /// event. A `Local` rather than anything new: it is a name bound over
    /// a body, which is what every other binder in the language is, so
    /// scoping, naming and emission all reuse the machinery that exists.
    pub payload: Option<LocalId>,
    /// Where the event name was written, for the diagnostic that has to
    /// name an event the language does not know.
    pub event_span: Span,
    pub body: BlockId,
    pub span: Span,
}
