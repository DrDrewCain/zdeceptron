//! The view, per spec §16.3.5.
//!
//! A region compiles to one static HTML string, parsed once into a
//! `<template>`, cloned per instantiation, walked to offsets computed here,
//! with a binding attached only at the holes. Nothing in this file consults
//! a type: the static/dynamic partition, the paths, the escaping and the
//! shape lookups are all reachability queries over what `zdc-resolve`
//! already produced, which is why the expensive machinery can land before
//! `zdc-types` exists and will not be rewritten when it does.
//!
//! The emitter never writes a newline or an indent inside a template
//! string. The HTML parser preserves inter-element whitespace as a text
//! node, and one such node shifts every subsequent `nextSibling` offset —
//! a failure with no compile-time signal, because the offsets simply point
//! at the wrong node (§16.10).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use zdc_hir::{
    DefId, DefKind, ExprId, HirArg, HirElement, HirExprKind, HirHandler, HirNode, HirNodeArmBody,
    Res,
};

use crate::elements::{self, Named, Slot};
use crate::expr::{Emitter, Literal, Operand};
use crate::js;
use crate::stmt::Statements;
use crate::style::{self, Declaration};
use crate::styles::Styles;

/// The parameter name the two-way sugar gives its own event.
///
/// It is not `$`-prefixed, because §16.3.6's table writes it as `e` and the
/// worked emissions in §16.4 are golden-tested against that byte for byte.
/// `names.rs` reserves it instead, so a program declaring `state e` is
/// emitted as `e$` and cannot be shadowed by this parameter — which it
/// silently was before.
const TWO_WAY_PARAMETER: &str = "e";

/// The event table's key for `NumberInput` and `DateInput`.
///
/// Not a DOM attribute name, unlike the other three keys: the write half
/// of these two is a property assignment rather than an attribute, so
/// there is no attribute for the key to double as. It is still routed
/// through `events.rs` so that what the listener reads comes from the one
/// table every other payload comes from.
const OPTIONAL_NUMBER_KEY: &str = "valueAsOptionalNumber";

/// A node of the static markup a region parses into.
#[derive(Debug, Clone)]
enum Tpl {
    Element {
        tag: &'static str,
        attributes: Vec<(String, String)>,
        children: Vec<Tpl>,
    },
    Text(String),
    /// One half of a hole's anchor pair. `each` and `when` do not know
    /// their contents at parse time, so the markup carries two comments and
    /// the runtime fills the gap between them (spec §16.3.5).
    Comment,
}

/// Where a node sits: an index into the region's roots, then one child
/// index per level down. Addresses are what the path scheduler turns into
/// `firstChild`/`nextSibling` chains.
type Address = Vec<usize>;

#[derive(Debug, Clone)]
enum BindKind {
    /// `bindText(node, getter)`.
    Text(String),
    /// One assignment at clone time; no effect is allocated.
    TextOnce(String),
    /// `markup(node, value)` — the one emitted call that parses HTML.
    ///
    /// Separate from [`BindKind::TextOnce`] rather than folded into it so
    /// that grepping the emitter for what can parse HTML finds exactly two
    /// constructors, both reachable only from `Slot::Rendered`.
    MarkupOnce(String),
    /// `bindMarkup(node, getter)`, for a markup value that can change.
    Markup(String),
    Attribute {
        name: String,
        getter: String,
    },
    AttributeOnce {
        name: String,
        value: String,
    },
    /// `$numberField(node, getter)` — the write half of `NumberInput` and
    /// `DateInput`.
    ///
    /// Not a [`BindKind::Attribute`] with the name `valueAsNumber`,
    /// because that would route through `dom.js`'s `setAttribute`, and
    /// what has to happen here is a *property* write guarded by a
    /// numeric comparison rather than a textual one. `intrinsics.rs`
    /// carries the helper and the argument for it, including why it is
    /// in a program's own preamble rather than in the shipped runtime.
    NumberField(String),
    /// `$modal(node, getter, setter)` — the whole of `Dialog`'s binding.
    ///
    /// One bind rather than a [`BindKind::Attribute`] plus a
    /// [`BindKind::Listener`], because the two halves are not independent:
    /// the effect must not call `showModal()` on a dialog the browser has
    /// already opened, and the `close` listener must not fight the effect
    /// that caused the close. Emitting them separately would put the rule
    /// that ties them together nowhere. `elements.rs`'s `Slot::Open` and
    /// the helper in `intrinsics.rs` state it.
    Modal {
        getter: String,
        setter: String,
    },
    Style {
        property: String,
        getter: String,
    },
    /// One `setProperty` at clone time; no effect is allocated.
    StyleOnce {
        property: String,
        value: String,
    },
    Listener {
        event: String,
        handler: String,
    },
    /// `eachInto(start, end, list, $byPosition, render)` — spec §16.3.9.
    Each {
        list: String,
        binder: String,
        body: Region,
    },
    /// `whenInto(start, end, scrutinee, arms)` — spec §16.3.8.
    When {
        scrutinee: String,
        arms: Vec<WhenArm>,
    },
    /// `ifInto(start, end, condition, then, otherwise)`.
    ///
    /// Not a `when`: there is no variant to name and no `choice` the
    /// program declared. Rebuilt only when the condition's truth changes,
    /// for the same reason `whenInto` rebuilds only on a tag change.
    If {
        condition: String,
        then: Region,
        otherwise: Option<Region>,
    },
    /// `foreign(node, create, props, declared)` — a `foreign … gives view`
    /// handed the `<div>` the template already carries (§14E.1).
    ///
    /// A *bind*, not a hole: the node exists in the static markup, so this
    /// sits beside `Attribute` and `Listener` rather than beside `Each`
    /// and `When`. That is the whole reason the form costs §16.2 R2
    /// nothing — an anchor pair would have needed the template model to
    /// grow a case for a region whose content the compiler never sees.
    /// `scene(node, {renderer, viewBox}, () => [...])` — a `<canvas>` and
    /// the drawing it paints, from `runtime/scene.js`.
    ///
    /// A bind rather than a hole, for the reason `Foreign` is one: the
    /// node is in the static markup and the compiler never has to walk
    /// inside it. What is inside is not DOM at all.
    Scene {
        renderer: String,
        view_box: String,
        draws: String,
    },
    Foreign {
        /// The local the `import` clause binds the export to.
        callee: String,
        /// One property per `takes` argument, in declaration order.
        props: Vec<(String, String)>,
        /// The declaration's name, as the program spells it.
        ///
        /// Emitted for the runtime's contract check and read by nothing
        /// else (#239). `mount(node, props) -> { update(props), destroy() }`
        /// is a shape no `foreign` declaration states and no type can hold,
        /// so the breach is discovered at mount — and a refusal that cannot
        /// name the declaration names a local out of this emitter instead,
        /// which is a worse report than the `TypeError` it replaces.
        declared: String,
    },
}

/// One `[read, write]` pair a region declares before it binds anything: a
/// component instance's own state (§14D.1).
#[derive(Debug, Clone)]
struct LocalDeclaration {
    getter: String,
    setter: Option<String>,
    is_source: bool,
    /// `every` / `after`, when the instance's clock writes this cell.
    ///
    /// This is the position that makes the cleanup rule observable: an
    /// instance is built inside `owned`, so the `onCleanup` the clock
    /// registers is torn down with the row or the `when` arm it belongs to.
    clock: Option<zdc_ast::Clock>,
    value: String,
}

/// One arm of a node-position `when`: its tag, its positional binders, and
/// the region it renders.
#[derive(Debug, Clone)]
struct WhenArm {
    name: String,
    binders: Vec<String>,
    body: Region,
}

#[derive(Debug, Clone)]
struct Bind {
    target: Address,
    kind: BindKind,
}

/// One arm of a `choice`, as a control has to offer it.
///
/// Two strings rather than one, because a variant now has two names: the
/// one the program matches on and the one a person reads. They are equal
/// unless the declaration said otherwise, and keeping them apart in the
/// type is what stops a label ever reaching the `value` attribute — where
/// it would make two arms sharing a label indistinguishable on the way
/// back through `variant`.
#[derive(Debug, Clone)]
struct Offered {
    tag: String,
    shown: String,
}

/// One template's worth of markup and the bindings attached to it.
#[derive(Debug, Clone)]
pub struct Region {
    roots: Vec<Tpl>,
    binds: Vec<Bind>,
    /// Signals declared once per *instance* of this region.
    ///
    /// A region is instantiated exactly where a component instance is: the
    /// root region once, an `each` body once per row, a `when` arm once per
    /// tag change. Declaring component state here is therefore what makes
    /// it per instance without any bookkeeping — two rows are two closures,
    /// so they are two signals.
    locals: Vec<LocalDeclaration>,
    /// `on key "…"` handlers, as `(key, handler source)`.
    ///
    /// Beside `locals` rather than in `binds`, because a bind is attached
    /// to a node and this is attached to nothing: the listener is on the
    /// document, and what belongs to *this* region is only its **lifetime**.
    /// That is exactly why it is a region member — a region is instantiated
    /// and discarded as a unit, so `onKey` registering `onCleanup` inside
    /// the closure is what makes a listener die with the branch that wrote
    /// it, using the disposal `ifInto`, `whenInto` and `eachInto` already
    /// perform.
    keys: Vec<(String, String)>,
}

impl Region {
    /// The static HTML this region parses from.
    pub fn html(&self) -> String {
        let mut out = String::new();
        for root in &self.roots {
            print_markup(root, &mut out);
        }
        out
    }

    /// Whether this region's markup has to be parsed inside an `<svg>`.
    ///
    /// **`<svg>` itself does not count.** A complete `<svg>…</svg>` is a
    /// tag the HTML parser knows on sight, and everything under it is
    /// namespaced by being under it — so the ordinary `template` handles
    /// the whole of an `Svg` element and its subtree. What needs help is a
    /// *row*: `each ring in rings` inside a drawing emits a region whose
    /// root is `<path>`, and that reaches the parser with nothing around
    /// it. Narrowing the rule here is what keeps `vector.js` out of a
    /// program whose drawing is written straight down.
    ///
    /// Structural rather than a look at the emitted string: the roots are
    /// still `Tpl` here, and a rule that read the markup back would be
    /// answering a question it had just finished writing down.
    pub fn is_svg(&self) -> bool {
        self.roots.iter().any(|root| match root {
            Tpl::Element { tag, .. } => *tag != "svg" && elements::is_svg_tag(tag),
            _ => false,
        })
    }

    /// Whether this region is one hole and nothing else.
    ///
    /// Such a region has no markup worth parsing, so `anchors()` builds its
    /// two comments directly rather than cloning a template made of them
    /// (spec §16.3.5 P2).
    fn is_only_anchors(&self) -> bool {
        self.roots.len() == 2 && self.roots.iter().all(|root| matches!(root, Tpl::Comment))
    }
}

/// The runtime symbols an emission used, so the import list can be narrowed.
#[derive(Default, Clone)]
pub struct RuntimeImports {
    pub signal: BTreeSet<&'static str>,
    pub dom: BTreeSet<&'static str>,
    /// The client half of the derived boundary. Present only when the
    /// split found a crossing, so a client-only program still imports
    /// nothing it does not use (§16.3.1).
    pub rpc: BTreeSet<&'static str>,
    /// Live sync. Present only when the split found a `durable` key —
    /// a program that reads a `server` signal and no durable one has
    /// nothing to keep in sync between windows.
    pub store: BTreeSet<&'static str>,
    /// The `foreign … gives view` lifecycle, from `runtime/foreign.js`.
    ///
    /// Separate from `dom` because it is a separate file, and it is a
    /// separate file because a program writing no DOM-owning foreign must
    /// not ship its create/update/destroy machinery (§16.3.1). Named for
    /// what it holds rather than for the module, so that it does not read
    /// as a second spelling of `foreign` below — that field is the *user*
    /// modules an emission imported, this one is a runtime symbol set like
    /// `signal` and `dom`.
    pub lifecycle: BTreeSet<&'static str>,
    /// The `Prose` render path, from `runtime/markup.js`.
    ///
    /// Separate from `dom` for the reason `lifecycle` is: `Prose` is the
    /// only element with a rendered slot, and a program without one must
    /// not ship an HTML parser call it never makes (§16.3.1).
    pub rendered: BTreeSet<&'static str>,
    /// Document key listeners, from `runtime/keys.js`.
    ///
    /// Separate from `dom` for the reason `lifecycle`, `rendered` and
    /// `reconcile` are, and with a sharper edge than any of them: the size
    /// gate's headroom is measured in hundreds of bytes, and this module
    /// carries the one thing in the runtime that decides whether a
    /// keystroke is a character somebody is typing. A program with no `on
    /// key` must ship neither (§16.3.1).
    pub keys: BTreeSet<&'static str>,
    /// Keyed list reconciliation, from `runtime/list.js`.
    ///
    /// Separate from `dom` for the reason `lifecycle` and `rendered` are:
    /// a program with no `each` must not ship a reconciler it never calls
    /// (§16.3.1), and the minimal-move reconciler is the largest single
    /// thing the renderer contains. Named for what it does rather than for
    /// the module, so it does not read as a second spelling of the `list`
    /// a program writes.
    pub reconcile: BTreeSet<&'static str>,
    /// The clock constructors, from `runtime/clock.js`.
    ///
    /// Separate from `signal` for the reason `reconcile` is separate from
    /// `dom`, and here the reason has a number attached: `signal.js` is in
    /// every bundle including the null program, so anything added to it is
    /// measured by the null-program size gate and paid for by every
    /// program forever. A program with no `every` and no `after` links
    /// none of this.
    pub clock: BTreeSet<&'static str>,
    /// The outbound request, from `runtime/request.js` (#19).
    ///
    /// Separate from `rpc` for the reason `lifecycle`, `rendered` and
    /// `reconcile` are separate from `dom`: a program that declares no
    /// `request` must not ship the one `fetch` in the runtime that can
    /// name a host, and separating the files is what makes "a bundle
    /// cannot reach a third party unless the program said so" a fact
    /// about the bytes rather than about a code path.
    pub request: BTreeSet<&'static str>,
    /// Every cross-origin destination this module's `request`
    /// declarations name, as a `connect-src` source expression (#19).
    ///
    /// Collected at the emission site — the same match arm that writes the
    /// `$request(…)` call — rather than walked out of the HIR separately,
    /// so "the origin in the policy is the origin the emitted code
    /// fetches" is one decision instead of two that have to agree. A
    /// same-origin destination adds nothing: `connect-src 'self'` already
    /// covers it, and naming it again would widen the policy to say what
    /// it already said.
    pub connect: BTreeSet<String>,
    /// The `foreign` declarations this module actually called, and the
    /// `import` each one needs: definition, module specifier, export.
    ///
    /// Keyed by definition so a foreign called twice is imported once, and
    /// collected during emission rather than from the HIR so a declaration
    /// nothing calls is not linked — §14E.2's "linked into whichever
    /// bundles actually call it", which is what keeps a `client` library
    /// out of a server bundle without a separate configuration.
    pub foreign: BTreeMap<DefId, (String, String)>,
    /// The `remembered` placement's store, from `runtime/remembered.js`.
    ///
    /// Separate from `signal` because it is a separate file, and it is a
    /// separate file for the reason `lifecycle`, `rendered` and
    /// `reconcile` are: a program that declares no `remembered` state must
    /// not download a `localStorage` wrapper (§16.3.1).
    pub remembered: BTreeSet<&'static str>,
    /// The browser-media reader, from `runtime/media.js`.
    ///
    /// Separate from `dom` for the reason `lifecycle`, `rendered` and
    /// `reconcile` are: a program that asks the browser nothing must not
    /// ship a `matchMedia` subscription it never installs (§16.3.1). The
    /// *queries* live on the emitter — they are per module, and this is a
    /// symbol set like `signal` and `dom`.
    pub media: BTreeSet<&'static str>,
    /// The viewport reader, from `runtime/viewport.js`.
    ///
    /// Its own set for the reason `media` has one: a program that never
    /// asks where the reader is must not ship a scroll listener it never
    /// installs (§16.3.1).
    pub viewport: BTreeSet<&'static str>,
    /// The scene rasterisers, from `runtime/scene.js`.
    ///
    /// Its own set for the reason `viewport` has one, and with more at
    /// stake: this is the largest module the runtime ships, and a program
    /// that draws with `Svg` rather than `Scene` must not download a WGSL
    /// shader and an ear clipper it never runs (§16.3.1).
    pub scene: BTreeSet<&'static str>,
    /// `templateSvg`, from `runtime/vector.js`.
    ///
    /// Asked for per *fragment* rather than per element: a program can
    /// draw an `Svg` whose whole subtree is one template and never need
    /// it, because a complete `<svg>…</svg>` parses unaided. What needs
    /// it is a row — an `each` inside a drawing — and that is the only
    /// thing that puts a name in this set.
    pub vector: BTreeSet<&'static str>,
    /// The `$`-prefixed prelude helpers this module used (§17.4.7).
    ///
    /// Not an import: §16.3.12 assertion A requires a bundle to import no
    /// ZDeceptron-generated module, so these are declared inline in the
    /// preamble — which is also what lets a program that never indexes a
    /// map ship without `$mapAt`.
    pub helpers: BTreeSet<&'static str>,
}

impl RuntimeImports {
    /// Fold another root's symbols into these.
    ///
    /// One emitter serves every root and its sets are cumulative, which is
    /// right for the client's import list and wrong for a root that has to
    /// *declare* what it reached. Those roots emit into an empty set and
    /// fold the result back here, so each gets its own share and the
    /// running total is still whole. A difference would not do: two
    /// endpoints that both construct a variant would leave the second's
    /// difference empty, and the second would declare nothing.
    /// **Every set, not the ones that existed when this was written.** A
    /// set left out here is one a folded root reached and the bundle then
    /// did not import — which is a `ReferenceError` on load rather than a
    /// missing optimisation, because `linked_runtime` reads these same
    /// sets to decide which files ship.
    pub fn absorb(&mut self, other: &RuntimeImports) {
        self.signal.extend(other.signal.iter().copied());
        self.dom.extend(other.dom.iter().copied());
        self.lifecycle.extend(other.lifecycle.iter().copied());
        self.rendered.extend(other.rendered.iter().copied());
        self.reconcile.extend(other.reconcile.iter().copied());
        self.clock.extend(other.clock.iter().copied());
        self.keys.extend(other.keys.iter().copied());
        self.rpc.extend(other.rpc.iter().copied());
        self.store.extend(other.store.iter().copied());
        self.remembered.extend(other.remembered.iter().copied());
        self.media.extend(other.media.iter().copied());
        self.request.extend(other.request.iter().copied());
        self.connect.extend(other.connect.iter().cloned());
        self.foreign
            .extend(other.foreign.iter().map(|(k, v)| (*k, v.clone())));
        self.helpers.extend(other.helpers.iter().copied());
    }
}

// --- P1 and P2: lowering and partition ------------------------------------

pub struct Lowering<'a, 'h> {
    emitter: &'a mut Emitter<'h>,
    styles: &'a mut Styles,
    binds: Vec<Bind>,
    locals: Vec<LocalDeclaration>,
    /// `on key "…"` handlers written directly in this region.
    keys: Vec<(String, String)>,
    /// How many sectioning elements enclose this point, which is what a
    /// `Heading` here becomes. Threaded rather than looked up because a
    /// `when` arm or an `each` body is a separate region: without carrying
    /// it, a heading inside a list inside a section would restart at `h1`.
    depth: usize,
    /// The built-in this point is written directly inside, so `Item`
    /// outside a `List` is a diagnostic rather than an orphaned `<li>`.
    parent: Option<&'static str>,
    /// Every signal a `PasswordInput` in this view binds.
    ///
    /// Collected from the whole node tree before anything is lowered,
    /// because the field may be written after the element that would leak
    /// it, and threaded into every sub-region, because a `when` arm and an
    /// `each` body are separate regions and a password shown in one is a
    /// password shown. See [`Lowering::check_masked`].
    masked: BTreeSet<DefId>,
    /// The URL this document is served at (#142).
    ///
    /// A `Link` whose destination is this exact URL is the link to the page
    /// the reader is already on. Sighted readers are told so by whatever
    /// the design does to the current item; nothing tells anyone else
    /// unless the program says so, and a routed program should not have to
    /// say twice what the address fold already knows.
    /// So it is a default the emitter adds, out of a fact the address fold
    /// already established: the destination renders to a compile-time
    /// literal, and the document's own URL is known while it is emitted.
    /// The `current` argument still wins where it is written: it is
    /// lowered after the leading one, so a self-link that means `step`
    /// says `step`.
    page_url: Option<&'a str>,
}

impl<'a, 'h> Lowering<'a, 'h> {
    pub fn new(
        emitter: &'a mut Emitter<'h>,
        styles: &'a mut Styles,
        page_url: Option<&'a str>,
    ) -> Lowering<'a, 'h> {
        Lowering {
            emitter,
            styles,
            binds: Vec::new(),
            locals: Vec::new(),
            keys: Vec::new(),
            depth: 0,
            parent: None,
            masked: BTreeSet::new(),
            page_url,
        }
    }

    pub fn region(mut self, nodes: &[HirNode]) -> Region {
        masked_signals(self.emitter.hir, nodes, &mut self.masked);
        let mut path = Vec::new();
        let roots = self.nodes(nodes, &mut path, 0);
        Region {
            roots,
            binds: self.binds,
            locals: self.locals,
            keys: self.keys,
        }
    }

    /// Lower a run of sibling nodes whose first markup child index is
    /// `start` — the slot text a `Button` carries has already taken index 0.
    fn nodes(&mut self, nodes: &[HirNode], path: &mut Address, start: usize) -> Vec<Tpl> {
        let mut out = Vec::new();
        for node in nodes {
            match node {
                HirNode::Element(element) => {
                    path.push(start + out.len());
                    let lowered = self.element(element, path);
                    path.pop();
                    out.push(lowered);
                }
                HirNode::Handler(handler) => match &handler.target {
                    // A document key handler belongs *here*, at node
                    // position, and nowhere else: it listens to no element,
                    // so writing it under one would say something the
                    // language does not mean. What its position does decide
                    // is its lifetime, which is the region it is written in.
                    zdc_hir::HandlerTarget::Document { key, .. } => {
                        let source = self.handler_source(handler);
                        self.keys.push((key.clone(), source));
                    }
                    zdc_hir::HandlerTarget::Element => self.emitter.error(
                        "`on` must be written inside the element it handles, indented under it.",
                        handler.span,
                    ),
                },
                HirNode::Each(each) => {
                    let target = hole(path, start + out.len(), &mut out);
                    // The list is a getter, so `eachInto` re-runs on a
                    // write; the binder is a getter too, because the row
                    // outlives any one version of its item (§16.3.9, R1).
                    let list = getter_source(self.emitter.operand(each.iter));
                    let binder = self.emitter.names.local(each.var).to_string();
                    let body = self.sub_region(&each.body);
                    self.bind(target, BindKind::Each { list, binder, body });
                }
                HirNode::When(when) => {
                    let target = hole(path, start + out.len(), &mut out);
                    // Bare, never `() => x()`: `read` unwraps exactly one
                    // level, and a thunk around a getter hands `whenInto` a
                    // function whose `.tag` is `undefined` (§16.3.8).
                    let scrutinee = getter_source(self.emitter.operand(when.scrutinee));
                    let mut arms = Vec::with_capacity(when.arms.len());
                    for arm in &when.arms {
                        // Exactly one parameter per declared field, so
                        // `Function.prototype.length` is the variant's
                        // arity — a contract `whenInto` relies on.
                        let binders: Vec<String> = arm
                            .bindings
                            .iter()
                            .map(|binding| self.emitter.names.local(*binding).to_string())
                            .collect();
                        let body = match &arm.body {
                            HirNodeArmBody::Show(element) => {
                                self.sub_region(&[HirNode::Element((**element).clone())])
                            }
                            HirNodeArmBody::Nodes(nodes) => self.sub_region(nodes),
                        };
                        arms.push(WhenArm {
                            name: arm.pattern_name.clone(),
                            binders,
                            body,
                        });
                    }
                    self.bind(target, BindKind::When { scrutinee, arms });
                }
                HirNode::If(conditional) => {
                    let target = hole(path, start + out.len(), &mut out);
                    let condition = getter_source(self.emitter.operand(conditional.cond));
                    let then = self.sub_region(&conditional.then);
                    let otherwise = conditional
                        .otherwise
                        .as_ref()
                        .map(|nodes| self.sub_region(nodes));
                    self.bind(
                        target,
                        BindKind::If {
                            condition,
                            then,
                            otherwise,
                        },
                    );
                }
                // One component instance. Its state is declared in *this*
                // region, so it is per instance without a wrapper element
                // and without a region boundary the program never wrote.
                HirNode::Scope(scope) => {
                    for local in &scope.locals {
                        let declaration = self.local_signal(local);
                        self.locals.push(declaration);
                    }
                    let lowered = self.nodes(&scope.body, path, start + out.len());
                    out.extend(lowered);
                }
                // Instantiation replaced every one of these already, so
                // reaching one means a component body was emitted directly.
                // unreached: `zdc-resolve` reports this first, in its own
                // words.
                HirNode::Children(span) => self.emitter.error(
                    "`children` can only be written inside a `component`, where it stands for the \
                     nodes nested under the call site.",
                    *span,
                ),
            }
        }
        out
    }

    /// A component instance's own `state`, as the pair it is emitted as.
    fn local_signal(&mut self, local: &zdc_hir::LocalSignal) -> LocalDeclaration {
        let value = self.emitter.value(local.init).into_text();
        LocalDeclaration {
            getter: self.emitter.names.local(local.local).to_string(),
            setter: self
                .emitter
                .names
                .local_setter(local.local)
                .map(str::to_string),
            is_source: local.is_source,
            clock: local.clock,
            value,
        }
    }

    /// A region nested inside this one: an `each` body or a `when` arm.
    ///
    /// It gets its own template and its own bind list, which is what §16.3.5
    /// P2 means by cutting at every hole.
    fn sub_region(&mut self, nodes: &[HirNode]) -> Region {
        Lowering {
            emitter: self.emitter,
            styles: self.styles,
            binds: Vec::new(),
            locals: Vec::new(),
            keys: Vec::new(),
            depth: self.depth,
            parent: self.parent,
            masked: self.masked.clone(),
            // A sub-region is part of the same document, so the URL it is
            // emitted for is the same one (#142).
            page_url: self.page_url,
        }
        .region(nodes)
    }

    fn element(&mut self, element: &HirElement, path: &mut Address) -> Tpl {
        // A `foreign … gives view` is written in element position and has
        // no entry in the shape table, so it is settled before the lookup
        // that would otherwise report the table and the resolver as having
        // drifted apart.
        if let Res::Def(def) = element.res {
            if matches!(self.emitter.hir.defs[def].kind, DefKind::Foreign(_)) {
                return self.foreign_element(def, element, path);
            }
        }
        let Some(shape) = elements::shape(&element.name) else {
            // unreached: An internal guard on §16.3.6's two tables.
            // `tests/element_parity.rs` fails first if `BUILT_INS` and `shape`
            // disagree.
            self.emitter.error(
                format!(
                    "`{}` has no DOM shape in the compiler's table, though the resolver accepted \
                     it. The two lists have drifted (spec §16.3.6).",
                    element.name
                ),
                element.span,
            );
            return Tpl::Text(String::new());
        };

        self.check_placement(element, &shape);
        self.check_masked(element);

        // A heading's level is its nesting depth, so an outline can neither
        // skip a level nor start below `h1`. Nothing in the program names
        // the level, which is what makes it impossible to write wrongly.
        let tag = if shape.heading {
            elements::heading_tag(self.depth)
        } else {
            shape.tag
        };

        // `Checkbox label is ...` wraps the box in a labelled row, so every
        // binding on the box sits one level below the element's own address.
        let labelled = matches!(shape.slot, Slot::Checked | Slot::Group)
            && named_argument_of(element, "label").is_some();
        let inner: Address = if labelled {
            let mut inner = path.clone();
            inner.push(0);
            inner
        } else {
            path.clone()
        };

        let mut attributes: Vec<(String, String)> = shape
            .attributes
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        let mut classes: Vec<String> = shape.base_class.map(str::to_string).into_iter().collect();
        let mut declarations: Vec<Declaration> = Vec::new();
        // A `class` that is not a literal is **held** rather than bound
        // where it is read. It is emitted as one assignment over the whole
        // attribute — `base + value` — and the base is the element's class
        // list joined; but a style set folds into a generated class only
        // after the last argument has been read, so binding it in the loop
        // wrote a base with no style class in it and the assignment then
        // dropped the styles at clone time. §16.2 R6 makes that a visible
        // loss rather than a cosmetic one: `Column` and `Row` carry
        // `zd-col`/`zd-row` instead of inline styles, so the class
        // attribute *is* the styling.
        let mut deferred_class: Option<Operand> = None;
        let mut children: Vec<Tpl> = Vec::new();

        self.leading_argument(element, shape.slot, &inner, &mut children, &mut attributes);

        if let Some(literal) = shape.literal_text {
            children.push(Tpl::Text(literal.to_string()));
        }

        // An argument named twice has no answer to which one wins, and for
        // `class` the two do not even compete: the first is folded into the
        // markup and the second appends to it, so the pair was the one way
        // a program's own text reached the base of a generated getter.
        let mut given: Vec<&str> = Vec::new();
        for arg in &element.args {
            let HirArg::Named { name, value } = arg else {
                continue;
            };
            // The destination was emitted by the slot above. It carries
            // an attribute name so that a rule over URL-bearing attribute
            // names sees it, and `zdc-resolve` refuses a source-written
            // one, so it is never an argument the program named.
            if shape.slot == Slot::Destination && name == zdc_hir::DESTINATION_ARGUMENT {
                continue;
            }
            if given.contains(&name.as_str()) {
                self.emitter.error(
                    format!(
                        "`{}` is given `{name}` twice. Each argument takes one value.",
                        element.name
                    ),
                    element.span,
                );
                continue;
            }
            given.push(name.as_str());
            if !elements::accepts_argument(&shape, name) {
                self.emitter.error(
                    format!(
                        "`{}` has no `{name}` argument. It takes {}. The set is closed: an \
                         argument the compiler does not know would reach the DOM as an attribute \
                         of that name, and `onclick`, `style` and `srcdoc` are attribute names.",
                        element.name,
                        permitted_arguments(&shape)
                    ),
                    element.span,
                );
                continue;
            }
            // `label` on a `Checkbox` is the text of the `<label>` the box
            // is wrapped in, so it is consumed here and emitted below.
            // Everywhere else it accepts one it is an ordinary named
            // argument that reaches `aria-label`, because there is no text
            // beside the control to wrap.
            if name == "label" && labelled {
                continue;
            }
            // The variant a radio stands for was read by the slot, which
            // wrote it into `value` and into the `checked` binding.
            if name == "option" && shape.slot == Slot::Group {
                continue;
            }
            // A `Scene`'s two are read by the emitter and handed to
            // `scene.js`: one names the backend, the other the coordinate
            // space. Neither is an attribute, and a `<canvas viewBox>`
            // would be a lie a reader could inspect.
            if element.name == "Scene" && matches!(name.as_str(), "renderer" | "viewBox") {
                continue;
            }
            // `elements.js`'s `Checkbox` reads only `label` and drops every
            // other argument on the floor. Refusing beats emitting markup
            // the reference implementation would not produce.
            if shape.slot == Slot::Checked {
                self.emitter.error(
                    format!(
                        "`Checkbox` takes only `label`; `elements.js` ignores `{name}` rather than \
                         applying it."
                    ),
                    element.span,
                );
                continue;
            }
            let operand = self.emitter.operand(*value);
            if shape.slot == Slot::Message && name == "message" {
                self.text_child(operand, &mut children, &inner);
                continue;
            }
            self.named_argument(
                name,
                operand,
                element,
                &inner,
                &mut attributes,
                &mut classes,
                &mut declarations,
                &mut deferred_class,
            );
        }

        for required in shape.required_arguments {
            if named_argument_of(element, required).is_none() {
                // unreached: `zdc-types` reports this first, in these same
                // words — `infer.rs` carries a copy of the sentence.
                self.emitter.error(
                    format!("`{}` needs `{required} is …`.", element.name),
                    element.span,
                );
            }
        }

        // HTML gives `<canvas>` an intrinsic 300x150 whatever box it is
        // in, so a `Scene` with no rule silently draws at a size nothing
        // in the program asked for — and the backend, which reads
        // `clientWidth` to size its backing store, rasterises at that size
        // too. The four declarations go in *here*, ahead of the program's
        // own, rather than into `base.css`: a rule in the shared sheet
        // would ship to every program that draws nothing, and the size
        // gate in `zdc-bench` is what makes that a build failure rather
        // than an opinion. Written first so a program's own `width is …`
        // lands after and wins.
        if element.name == "Scene" {
            let mut sizing = vec![
                ("display", "block"),
                ("width", "100%"),
                ("height", "100%"),
                ("min-height", "1px"),
            ]
            .into_iter()
            .map(|(property, value)| Declaration {
                condition: style::Condition::Always,
                property: property.to_string(),
                value: value.to_string(),
            })
            .collect::<Vec<_>>();
            sizing.append(&mut declarations);
            declarations = sizing;
        }

        // A static style set folds into one generated class and costs
        // nothing at runtime (spec §6, §16.3.11).
        if !declarations.is_empty() {
            classes.push(self.styles.intern(declarations));
        }
        if !classes.is_empty() {
            set_attribute(&mut attributes, "class", classes.join(" "));
        }
        // Now that the class list is whole, and not before.
        if let Some(operand) = deferred_class {
            self.class_binding(operand, &classes, &inner);
        }

        // Handlers are children in the HIR and listeners in the emission,
        // so they never reach the markup.
        let mut submits = false;
        for child in &element.children {
            if let HirNode::Handler(handler) = child {
                // Refused where it is written rather than emitted where it
                // would work. `on key` under a `Button` reads as "while
                // this button has focus", and it does not mean that — it
                // listens to the whole document however deeply it is
                // nested, so the two readings differ by every keystroke
                // aimed somewhere else.
                if let zdc_hir::HandlerTarget::Document { .. } = handler.target {
                    self.emitter.error(
                        format!(
                            "`on key` listens to the document, not to `{}`. Write it beside the \
                             element rather than indented under it.",
                            element.name
                        ),
                        handler.span,
                    );
                    continue;
                }
                submits |= handler.event == "submit";
                self.listener(element, shape.slot, handler, &inner);
            }
        }
        // A `form` with no submit handler navigates: the browser reloads
        // the current URL with the fields as a query string, and every
        // client signal on the page is gone. It fails at the one moment
        // somebody presses Enter, and it fails silently, so it is refused
        // where the form is written.
        if element.name == "Form" && !submits {
            self.emitter.error(
                "`Form` needs `on submit`, written indented under it. Without one, pressing Enter \
                 in any field navigates the browser away and every value on the page is lost.",
                element.span,
            );
        }

        let element_children: Vec<HirNode> = element
            .children
            .iter()
            .filter(|child| !matches!(child, HirNode::Handler(_)))
            .cloned()
            .collect();
        if element_children.is_empty() {
            self.check_leading_child(element, &shape, &[]);
        }
        if element.name == "Scene" {
            self.check_only_children(element, &shape, &element_children);
            self.scene(element, &element_children, &inner);
            return Tpl::Element {
                tag,
                attributes,
                children: Vec::new(),
            };
        }
        if !element_children.is_empty() {
            if shape.children {
                self.check_only_children(element, &shape, &element_children);
                self.check_leading_child(element, &shape, &element_children);
                // Inside the wrapper when there is one, and the wrapper is
                // then the element's only child, so the children start at
                // index 0 of a path one level deeper.
                let mut child_path = inner.clone();
                let start = if shape.inner_tag.is_some() {
                    child_path.push(0);
                    0
                } else {
                    children.len()
                };
                let outer_depth = self.depth;
                let outer_parent = self.parent;
                if shape.sectioning {
                    self.depth += 1;
                }
                self.parent = shape_name(&element.name);
                let lowered = self.nodes(&element_children, &mut child_path, start);
                self.depth = outer_depth;
                self.parent = outer_parent;
                children.extend(lowered);
            } else {
                self.emitter.error(
                    format!("`{}` shows one value and takes no children.", element.name),
                    element.span,
                );
            }
        }

        // The row group a `Table` writes, so the markup parses into the
        // tree this emitter built rather than into the one the HTML
        // parser would have inserted around it.
        if let Some(wrapper) = shape.inner_tag {
            children = vec![Tpl::Element {
                tag: wrapper,
                attributes: Vec::new(),
                children,
            }];
        }

        let node = Tpl::Element {
            tag,
            attributes,
            children,
        };
        if !labelled {
            return node;
        }

        let mut label_children = vec![node];
        if let Some(value) = named_argument_of(element, "label") {
            let operand = self.emitter.operand(value);
            self.text_child(operand, &mut label_children, path);
        }
        Tpl::Element {
            tag: "label",
            attributes: vec![(
                "class".to_string(),
                elements::CHECKBOX_LABEL_CLASS.to_string(),
            )],
            children: label_children,
        }
    }

    /// A `Scene`'s children, as the draw list `scene.js` paints.
    ///
    /// # Why this is an expression and not a region
    ///
    /// Everywhere else in this emitter, children become DOM nodes and a
    /// list or a conditional becomes an *anchor pair* with a region
    /// between them, because the browser keeps the nodes and the emitter
    /// has to say which ones to replace. A canvas keeps nothing. There is
    /// no node to reconcile, so `each` and `if` lower to `flatMap` and a
    /// ternary — plain JavaScript expressions inside one thunk — and the
    /// whole tree is rebuilt on any change.
    ///
    /// That is not a compromise. Rebuilding is what an immediate-mode
    /// renderer does anyway: the previous frame is gone. It also means a
    /// `Scene` allocates no anchors, no regions and no effects beyond the
    /// single one `scene()` runs, so forty thousand shapes cost forty
    /// thousand object literals and nothing else.
    ///
    /// # Why the thunk
    ///
    /// The list is passed unevaluated. `scene()` calls it inside `effect`,
    /// so every signal any shape reads is tracked by the ordinary
    /// mechanism and a write repaints — without this emitter working out
    /// which shapes depend on what, which is a dependency analysis the
    /// language deliberately does not have anywhere else either.
    fn scene(&mut self, element: &HirElement, children: &[HirNode], target: &Address) {
        let renderer = self.scene_option(element, "renderer", "'auto'");
        let view_box = self.scene_option(element, "viewBox", "'0 0 100 100'");
        let draws = self.draw_nodes(children);
        self.emitter.used.scene.insert("scene");
        self.bind(
            target.clone(),
            BindKind::Scene {
                renderer,
                view_box,
                draws,
            },
        );
    }

    /// One of a `Scene`'s two mount-time options, as a JavaScript value.
    ///
    /// Read once at mount and not per repaint, which is what makes a
    /// reactive one a read rather than a getter: neither the backend nor
    /// the coordinate space can change after the canvas exists, so a
    /// program that writes a signal into either gets its first value and
    /// nothing later. That is a narrowing worth stating and not worth a
    /// diagnostic — the two are configuration, and both are literals in
    /// every program written so far.
    fn scene_option(&mut self, element: &HirElement, name: &str, default: &str) -> String {
        let Some(expr) = named_argument_of(element, name) else {
            return default.to_string();
        };
        match self.emitter.operand(expr) {
            Operand::Literal(literal) => literal.as_js(),
            Operand::Static(value) => value,
            Operand::Reactive(getter) => format!("({getter})()"),
        }
    }

    /// A run of nodes as the elements of a JavaScript array literal.
    ///
    /// Every arm produces a *spread-safe* fragment: an object literal, or
    /// a `...` of something that is always an array. That is what lets a
    /// list of rings and a lone axis line sit beside each other in one
    /// literal with no concatenation.
    fn draw_nodes(&mut self, nodes: &[HirNode]) -> String {
        let mut parts: Vec<String> = Vec::new();
        for node in nodes {
            match node {
                HirNode::Element(element) => {
                    if let Some(part) = self.draw_element(element) {
                        parts.push(part);
                    }
                }
                HirNode::Each(each) => {
                    let list = getter_source(self.emitter.operand(each.iter));
                    let binder = self.emitter.names.local(each.var).to_string();
                    let body = self.draw_nodes(&each.body);
                    // The row is bound as a getter, exactly as `eachInto`
                    // binds one, so an expression that reads it emits the
                    // same call it would emit in a DOM row. One convention,
                    // two lowerings.
                    parts.push(format!(
                        "...({list})().flatMap(($row) => {{ const {binder} = () => $row; return [{body}]; }})"
                    ));
                }
                HirNode::If(conditional) => {
                    let condition = getter_source(self.emitter.operand(conditional.cond));
                    let then = self.draw_nodes(&conditional.then);
                    let otherwise = conditional
                        .otherwise
                        .as_ref()
                        .map(|nodes| self.draw_nodes(nodes))
                        .unwrap_or_default();
                    parts.push(format!("...(({condition})() ? [{then}] : [{otherwise}])"));
                }
                // A component with no state of its own is its body, so it
                // splices. One that declares `state` would need a cell per
                // instance, and there is no instance here to hang it on —
                // the shapes are values in an array, not nodes with a
                // lifetime — so it is refused rather than silently shared.
                HirNode::Scope(scope) => {
                    if scope.locals.is_empty() {
                        parts.push(self.draw_nodes(&scope.body));
                    } else {
                        self.emitter.error(
                            "A component used inside a `Scene` cannot declare `state`: a drawn \
                             shape is a value in a list, with no instance to keep a cell on. \
                             Lift the state to the program that draws the scene.",
                            scope.span,
                        );
                    }
                }
                HirNode::When(when) => self.emitter.error(
                    "`when` cannot be written inside a `Scene` yet. Use `if`, or match outside \
                     the scene and draw each arm's shapes there.",
                    when.span,
                ),
                HirNode::Handler(handler) => self.emitter.error(
                    "A `Scene` is painted, not laid out, so there is no node under it to listen \
                     to. Write the handler on an element around the scene.",
                    handler.span,
                ),
                // unreached: instantiation replaced every one already.
                HirNode::Children(span) => self.emitter.error(
                    "`children` can only be written inside a `component`.",
                    *span,
                ),
            }
        }
        parts.join(", ")
    }

    /// One shape, as the object literal `scene.js` reads.
    fn draw_element(&mut self, element: &HirElement) -> Option<String> {
        let op = match element.name.as_str() {
            "Group" => "group",
            "Path" => "path",
            "Circle" => "circle",
            "Segment" => "line",
            _ => {
                // unreached: `only_children` on `Scene` and `Group` answers
                // first, naming the four by name.
                self.emitter.error(
                    format!("`{}` is not a shape a `Scene` can draw.", element.name),
                    element.span,
                );
                return None;
            }
        };
        // unreached: every one of the four is in the table.
        let shape = elements::shape(&element.name)?;
        let mut fields = vec![format!("op: {}", js::string(op))];
        let mut given: Vec<&str> = Vec::new();
        for arg in &element.args {
            let HirArg::Named { name, value } = arg else {
                continue;
            };
            given.push(name.as_str());
            // The permitted set here is the shape's own and *not*
            // `permitted_arguments`, which adds every global argument there
            // is. `class`, `id` and `role` are real on a DOM node and mean
            // nothing to a rasteriser, so listing them would offer the
            // writer sixty repairs that do not work.
            let permitted = english_list(shape.arguments);
            let Some(field) = draw_field(name).filter(|_| elements::accepts_argument(&shape, name))
            else {
                self.emitter.error(
                    format!(
                        "`{}` has no `{name}` argument inside a `Scene`. It takes {permitted}.",
                        element.name
                    ),
                    element.span,
                );
                continue;
            };
            // Read, not bound: the whole literal is rebuilt inside the
            // repaint effect, so a getter is called here and the effect
            // that surrounds it is what records the dependency.
            let source = match self.emitter.operand(*value) {
                Operand::Literal(literal) => literal.as_js(),
                Operand::Static(value) => value,
                Operand::Reactive(getter) => format!("({getter})()"),
            };
            fields.push(format!("{field}: {source}"));
        }
        // Worded apart from the identical rule in `element()` on purpose.
        // Both are guards on `zdc-types`' copy of the same table, which
        // reports first; two sites sharing one sentence would read as one
        // rule firing twice, and the diagnostic-coverage test attributes
        // by message text, so it could not tell them apart either.
        for required in shape.required_arguments {
            if !given.contains(required) {
                // unreached: `zdc-types` reports this first, in nearly
                // these words — `elements.rs` there carries the required
                // list for the vector family.
                self.emitter.error(
                    format!("`{}` needs `{required} is …` to be drawn.", element.name),
                    element.span,
                );
            }
        }
        if op == "group" {
            let children = self.draw_nodes(&element.children);
            fields.push(format!("children: [{children}]"));
        } else if let Some(handler) = element.children.iter().find_map(|child| match child {
            HirNode::Handler(handler) => Some(handler),
            // Written out rather than `_`, so a new kind of node is a
            // compile error here instead of silently not being a handler.
            HirNode::Element(_)
            | HirNode::Each(_)
            | HirNode::When(_)
            | HirNode::If(_)
            | HirNode::Children(_)
            | HirNode::Scope(_) => None,
        }) {
            // Ahead of the children rule, because a handler *is* a child in
            // the HIR and "takes no children" would send the writer looking
            // for a shape they did not write.
            self.emitter.error(
                "A `Scene` is painted, not laid out, so there is no node under it to listen to. \
                 Write the handler on an element around the scene.",
                handler.span,
            );
        } else if !element.children.is_empty() {
            self.emitter.error(
                format!("`{}` draws one shape and takes no children.", element.name),
                element.span,
            );
        }
        Some(format!("{{ {} }}", fields.join(", ")))
    }

    /// A `foreign … gives view` written as a view element (§14E.1).
    ///
    /// The boundary in is two things and nothing else: a `<div>` the
    /// template carries, and a plain object with one property per `takes`
    /// argument in declaration order. The boundary out is nothing at all —
    /// the handle is consumed by the runtime and is never a ZDeceptron
    /// value — which is why §19.2 rule 12 has no question to ask here.
    ///
    /// The `<div>` is markup rather than an anchor pair on purpose. A hole
    /// whose contents the compiler never sees still has a *known extent*
    /// if it is an element, and an element is what §16.2 R2 already knows
    /// how to clone and walk to. The foreign then owns a subtree it cannot
    /// be confused about the edges of.
    fn foreign_element(&mut self, def: DefId, element: &HirElement, path: &mut Address) -> Tpl {
        let node = Tpl::Element {
            tag: "div",
            attributes: Vec::new(),
            children: Vec::new(),
        };
        let DefKind::Foreign(foreign) = self.emitter.hir.defs[def].kind.clone() else {
            unreachable!("the caller matched on `DefKind::Foreign`");
        };
        let declared = self.emitter.hir.defs[def].name.clone();

        // A `gives view` foreign is *called* by the runtime with a node
        // and a props object, so it is an import and never a method: there
        // is no receiver at a view element to look one up on. Refused in
        // name resolution, and guarded here because this is the site that
        // would otherwise record an import of nothing.
        let Some(module) = foreign.module().map(str::to_string) else {
            // unreached: `zdc-resolve` reports this first, in its own
            // words — a `gives view` foreign declaring `on Handle` is
            // refused at the declaration.
            self.emitter.error(
                format!(
                    "`{declared}` gives a view and is declared `on {}`. A view foreign is called \
                     by the runtime with a DOM node, so there is no receiver to look a method up \
                     on (spec §14E.1).",
                    zdc_ast::HANDLE_TYPE_NAME
                ),
                element.span,
            );
            return node;
        };
        // Only a real module is imported. A `zd:` specifier names the
        // language's own primitive layer, which is emitted inline and has
        // no DOM node to own, so a `gives view` on one is a prelude bug
        // rather than anything a program can write.
        if crate::intrinsics::intrinsic(&module, foreign.export.as_str()).is_some() {
            // unreached: the prelude declares every `zd:` primitive and
            // not one of them gives a view, so no program can write this.
            // A guard on the prelude rather than on a program.
            self.emitter.error(
                format!(
                    "`{declared}` gives a view and names a `zd:` primitive. The primitive layer \
                     is emitted inline and owns no DOM node (spec §17.4.7)."
                ),
                element.span,
            );
            return node;
        }
        // Checked again at the *emission* site, as the call path checks it:
        // the parser guards one construct's syntax, and this guards the
        // position the name is written into. Two passes, one rule.
        let symbol = foreign.export.as_str().to_string();
        if js::ident(&symbol).is_none() {
            // unreached: the parser reports this first, in its own words —
            // `foreign_export` refuses a literal that is not a JavaScript
            // identifier, so none survives to be emitted. Kept for the
            // same reason the call path keeps its copy.
            self.emitter.error(
                format!(
                    "`{declared}` would be imported as `{symbol}`, which is not a JavaScript \
                     identifier. An `import` clause needs a name as syntax, so there is no \
                     escaping that makes this safe (spec §14E.1)."
                ),
                element.span,
            );
            return node;
        }

        // A foreign owns its node and everything under it, so nodes written
        // inside would be discarded the moment the module touched its
        // subtree. Refused rather than dropped: silently emitting markup a
        // foreign is free to delete is the kind of thing that is noticed
        // only as "my button vanished sometimes".
        if !element.children.is_empty() {
            // unreached: `zdc-types` reports this first, in its own words.
            // Kept because this is the site that would otherwise emit
            // markup the foreign is free to delete.
            self.emitter.error(
                format!(
                    "`{declared}` gives a view, so it owns this node and everything inside it. \
                     Nothing can be written under it — including `on`, because the foreign \
                     decides what its subtree is (spec §14E.1)."
                ),
                element.span,
            );
            return node;
        }

        let names: Vec<String> = foreign
            .params
            .iter()
            .map(|param| self.emitter.hir.locals[*param].name.clone())
            .collect();
        let Some(ordered) = self.foreign_arguments(element, &declared, &names) else {
            return node;
        };

        // Read in declaration order, not in written order: the object the
        // module receives is the declaration's own shape, so a program
        // reordering its named arguments changes nothing on the far side.
        let props: Vec<(String, String)> = names
            .iter()
            .zip(ordered)
            .map(|(name, expr)| (name.clone(), self.emitter.value(expr).into_text()))
            .collect();

        // Recorded at the *use*, exactly as a call records it, so §14E.2's
        // "linked into whichever bundles actually call it" holds for this
        // form too — a declared-but-unwritten foreign is not imported.
        self.emitter.used.foreign.insert(def, (module, symbol));

        let callee = self.emitter.names.def(def).to_string();
        self.bind(
            path.clone(),
            BindKind::Foreign {
                callee,
                props,
                declared,
            },
        );
        node
    }

    /// The written arguments of a foreign element, in declaration order.
    ///
    /// `None` when the call does not match the declaration, with the
    /// diagnostic already reported. The matching is the same rule the call
    /// path applies — §14E.1 gives a foreign one parameter list whichever
    /// position it is written in.
    fn foreign_arguments(
        &mut self,
        element: &HirElement,
        declared: &str,
        names: &[String],
    ) -> Option<Vec<ExprId>> {
        let mut ordered: Vec<Option<ExprId>> = vec![None; names.len()];
        let mut next_positional = 0;
        for arg in &element.args {
            match arg {
                HirArg::Positional(expr) => {
                    if next_positional >= ordered.len() {
                        // unreached: `zdc-types` reports this first, and
                        // in better words — it pluralises the count.
                        self.emitter.error(
                            format!(
                                "`{declared}` takes {} argument(s), and this writes more.",
                                names.len()
                            ),
                            element.span,
                        );
                        return None;
                    }
                    ordered[next_positional] = Some(*expr);
                    next_positional += 1;
                }
                HirArg::Named { name, value } => match names.iter().position(|param| param == name)
                {
                    Some(index) => ordered[index] = Some(*value),
                    None => {
                        // unreached: `zdc-types` reports this first, in
                        // its own words.
                        self.emitter.error(
                            format!(
                                "`{declared}` has no parameter named `{name}`. Its parameters are \
                                 {}.",
                                names.join(", ")
                            ),
                            element.span,
                        );
                        return None;
                    }
                },
            }
        }
        let mut out = Vec::with_capacity(ordered.len());
        for (index, slot) in ordered.iter().enumerate() {
            match slot {
                Some(expr) => out.push(*expr),
                None => {
                    // unreached: `zdc-types` reports this first, in its
                    // own words.
                    self.emitter.error(
                        format!(
                            "`{declared}` is missing an argument for `{}`.",
                            names[index]
                        ),
                        element.span,
                    );
                    return None;
                }
            }
        }
        Some(out)
    }

    fn leading_argument(
        &mut self,
        element: &HirElement,
        slot: Slot,
        target: &Address,
        children: &mut Vec<Tpl>,
        attributes: &mut Vec<(String, String)>,
    ) {
        let mut positionals = element.args.iter().filter_map(|arg| match arg {
            HirArg::Positional(expr) => Some(*expr),
            HirArg::Named { .. } => None,
        });
        // A `Link`'s destination was written first and is held under the
        // name `href` (`zdc_hir::DESTINATION_ARGUMENT`), so that a rule
        // over URL-bearing attribute names sees it. It is still the
        // leading argument as far as the slot is concerned.
        let leading = zdc_hir::destination_of(element).or_else(|| positionals.next());
        if positionals.next().is_some() {
            // unreached: `zdc-types` reports this first, in its own words.
            self.emitter.error(
                format!("`{}` takes at most one leading argument.", element.name),
                element.span,
            );
        }

        match (slot, leading) {
            // unreached: `zdc-types` reports this first, in its own words. Its
            // sentence is about a leading *value* rather than a leading
            // *argument*, and it is the one a user sees.
            (Slot::None, Some(_)) => self.emitter.error(
                format!(
                    "`{}` takes no leading argument. Everything it shows is nested inside it: \
                     write the value as a child, such as `Text …`.",
                    element.name
                ),
                element.span,
            ),
            (Slot::Text | Slot::OptionalText, Some(expr)) => {
                let operand = self.emitter.operand(expr);
                self.text_child(operand, children, target);
            }
            // unreached: `zdc-types` reports this first, in its own words.
            (Slot::Text, None) => self.emitter.error(
                format!("`{}` needs the text it shows.", element.name),
                element.span,
            ),
            (Slot::OptionalText, None) => {}
            // The whole of the markup path, and it is four lines because
            // the type checker did the work: reaching here at all means
            // the argument's type is `Markup`, which only `build markdown`
            // produces (`zdc_types::Type::Markup`).
            (Slot::Rendered, Some(expr)) => {
                let operand = self.emitter.operand(expr);
                self.markup_child(operand, target);
            }
            // unreached: `zdc-types` reports this first, in its own words.
            // `Slot::Rendered` with no leading argument is refused by
            // `infer`'s element check, which also names `build markdown` as
            // the only thing that makes a `Markup` — the sentence a user
            // needs. This arm keeps the match total.
            (Slot::Rendered, None) => self.emitter.error(
                format!(
                    "`{}` needs the markup it renders, written first.",
                    element.name
                ),
                element.span,
            ),
            (Slot::Destination, Some(expr)) => {
                // One path for both kinds of destination. A route value
                // is rendered into its URL first (§14G.2's bijection),
                // and the URL then takes exactly the route a named URL
                // argument takes: the scheme filter, and `safeUrl` when
                // it is not known until run time.
                let operand = match self.route_destination(expr) {
                    Some(url) => url,
                    None => self.emitter.operand(expr),
                };
                self.mark_current_page(&operand, attributes);
                self.url_attribute(
                    zdc_hir::DESTINATION_ARGUMENT,
                    operand,
                    element,
                    target,
                    attributes,
                );
            }
            // unreached: `zdc-types` reports this first, in its own words.
            (Slot::Destination, None) => self.emitter.error(
                format!(
                    "`{}` needs somewhere to go, written first: `{} \"https://example.com\"`, or \
                     `{} Home` for one of this program's own routes.",
                    element.name, element.name, element.name
                ),
                element.span,
            ),
            (Slot::Value | Slot::Checked | Slot::Level, Some(expr)) => {
                // The DOM attribute and the event-table key are the same
                // name for two of the three, and differ for `Slider`:
                // `value` is what the browser is told and
                // `valueAsNumber` is what the listener reads back.
                let (attribute, bound) = match slot {
                    Slot::Value => ("value", "value"),
                    Slot::Level => ("value", "valueAsNumber"),
                    _ => ("checked", "checked"),
                };
                self.two_way(element, expr, attribute, bound, target);
            }
            // unreached: `zdc-types` reports this first, in its own words.
            (Slot::Value | Slot::Checked | Slot::Level, None) => self.emitter.error(
                format!("`{}` needs the state it binds to.", element.name),
                element.span,
            ),
            (Slot::OptionalLevel, Some(expr)) => self.optional_number(element, expr, target),
            // unreached: `zdc-types` reports this first, in its own words.
            (Slot::OptionalLevel, None) => self.emitter.error(
                format!("`{}` needs the state it binds to.", element.name),
                element.span,
            ),
            (Slot::Open, Some(expr)) => self.modal(element, expr, target),
            // unreached: `zdc-types` reports this first, in its own words.
            (Slot::Open, None) => self.emitter.error(
                format!(
                    "`{}` needs the state that says whether it is showing, written first. \
                     Nothing else opens or closes it.",
                    element.name
                ),
                element.span,
            ),
            // unreached: `zdc-types` reports this first, in its own words.
            (Slot::Message, Some(_)) => self.emitter.error(
                "`ErrorBar` takes its text as `message is ...`, not as a leading argument.",
                element.span,
            ),
            (Slot::Choice, Some(expr)) => self.choice(element, expr, target, children),
            (Slot::Group, Some(expr)) => self.radio(element, expr, target, attributes),
            // unreached: `zdc-types` reports this first, in its own words.
            (Slot::Group, None) => self.emitter.error(
                format!("`{}` needs the state it binds to.", element.name),
                element.span,
            ),
            // unreached: `zdc-types` reports this first, in its own words.
            (Slot::Choice, None) => self.emitter.error(
                format!("`{}` needs the state it binds to.", element.name),
                element.span,
            ),
            // A number, written into `value` and never read back. One
            // binding, no listener: this is a report rather than a
            // control, so it needs neither the event nor §14B.5's rule
            // about which signals a keystroke may write.
            (Slot::Amount, Some(expr)) => {
                let operand = self.emitter.operand(expr);
                match operand {
                    Operand::Literal(literal) => {
                        set_attribute(attributes, "value", literal.as_text())
                    }
                    Operand::Static(value) => self.bind(
                        target.clone(),
                        BindKind::AttributeOnce {
                            name: "value".to_string(),
                            value,
                        },
                    ),
                    Operand::Reactive(getter) => self.bind(
                        target.clone(),
                        BindKind::Attribute {
                            name: "value".to_string(),
                            getter,
                        },
                    ),
                }
            }
            // unreached: `zdc-types` reports this first, in its own words.
            (Slot::Amount, None) => self.emitter.error(
                format!("`{}` needs the number it shows.", element.name),
                element.span,
            ),
            (Slot::None | Slot::Message, None) => {}
        }
    }

    /// A route value's URL, or `None` when the destination is not one.
    ///
    /// A constant URL is baked into the markup, exactly as any other
    /// constant attribute is: a link to `/writing` costs nothing at
    /// runtime, and a link inside an `each` becomes a binding because the
    /// row's slug is a getter.
    fn route_destination(&mut self, expr: ExprId) -> Option<Operand> {
        if !self.emitter.is_route_value(expr) {
            return None;
        }
        Some(self.emitter.route_url(expr))
    }

    #[allow(clippy::too_many_arguments)]
    fn named_argument(
        &mut self,
        name: &str,
        operand: Operand,
        element: &HirElement,
        target: &Address,
        attributes: &mut Vec<(String, String)>,
        classes: &mut Vec<String>,
        declarations: &mut Vec<Declaration>,
        deferred_class: &mut Option<Operand>,
    ) {
        // Two names that would reach the DOM and become something other
        // than an attribute.
        //
        // `style` is a CSS context. Escaping for markup is not escaping for
        // CSS any more than it is escaping for a URL, and a `url(…)` inside
        // one is a request the browser issues from a value that never looks
        // like a URL to a reader. The emitter already owns this attribute —
        // `padding` and `weight` fold into a generated class (§16.3.11) —
        // so there is nothing to give up by refusing it.
        //
        // `on…` is a script. Events are written `on click`, indented under
        // the element, which is a node the compiler can see into; an
        // `onclick` attribute is a program the compiler never parses.
        if name == "style" {
            // unreached: the closed argument set answers first — `style`
            // is not an argument any element accepts, so `an-argument-
            // outside-the-closed-set` is the refusal a program gets. Kept
            // because the closed set is per element and this is not.
            self.emitter.error(
                "A `style` argument is a CSS context, and the escaping the emitter does is for \
                 markup. Use `padding is …` and `weight is …`, which fold into a generated class \
                 (spec §16.3.5, §16.3.11).",
                element.span,
            );
            return;
        }
        if zdc_hir::is_event_attribute(name) {
            // unreached: the closed argument set answers first, for the
            // same reason `style` above does. Kept because this ranges
            // over every `on…` spelling rather than over one table.
            self.emitter.error(
                format!(
                    "`{name}` would install a script as an attribute. Write `on {}` indented \
                     under the element instead (spec §16.3.7).",
                    name.strip_prefix("on").unwrap_or(name)
                ),
                element.span,
            );
            return;
        }
        let Some(named) = elements::named_argument(&element.name, name) else {
            // unreached: An internal guard on the other half of §16.3.6.
            // `named_arguments_are_total` fails first if an accepted argument
            // has no meaning.
            self.emitter.error(
                format!(
                    "`{name}` is accepted by `{}` but has no DOM meaning in the compiler's table. \
                     The two halves of §16.3.6 have drifted.",
                    element.name
                ),
                element.span,
            );
            return;
        };
        // Every name the browser dereferences is filtered, whichever arm
        // of the table it reaches (`zdc_hir::URL_ATTRIBUTES`). The table
        // routes the ones it knows about to `Named::Url`; this catches a
        // name that is URL-bearing and is spelled as a plain attribute.
        let named = match named {
            Named::Attribute(attribute) if zdc_hir::is_url_attribute(name) => Named::Url(attribute),
            other => other,
        };
        match named {
            Named::Url(attribute) => {
                self.url_attribute(attribute, operand, element, target, attributes)
            }
            Named::State(attribute) => self.state_attribute(attribute, operand, target, attributes),
            Named::Word(attribute, words) => {
                self.word_attribute(name, attribute, words, operand, element, attributes)
            }
            // unreached: `Named::Consumed` is `label` and `message`, and both
            // are answered above this call rather than reaching it.
            Named::Consumed => self.emitter.error(
                format!("`{}` does not use `{name}`.", element.name),
                element.span,
            ),
            // A literal joins the class list here; anything else is held
            // until the list is whole, because the assignment it becomes
            // replaces the attribute rather than adding to it.
            Named::Class => match operand {
                Operand::Literal(literal) => classes.push(literal.as_text()),
                // Held, not bound here: the class list is not whole until
                // the folded style class has joined it, and the assignment
                // this becomes replaces the attribute rather than adding to
                // it. `class_binding` does the binding, and it builds the
                // base with `js::string` rather than interpolating it into
                // a JavaScript literal — a program can put its own literal
                // among these classes, and `class is "a'+alert(1)+'b"`
                // would otherwise close the quote and write expressions
                // into the emitted module.
                Operand::Static(value) => *deferred_class = Some(Operand::Static(value)),
                Operand::Reactive(getter) => *deferred_class = Some(Operand::Reactive(getter)),
            },
            Named::Style(argument) => {
                self.style_argument(name, argument, operand, element, target, declarations)
            }
            Named::Attribute(attribute) => match operand {
                // `setAttribute` removes on `false` and sets the empty
                // string on `true`, so a static one is markup or nothing.
                Operand::Literal(Literal::Truth(false)) => {}
                Operand::Literal(Literal::Truth(true)) => {
                    set_attribute(attributes, attribute, String::new());
                }
                Operand::Literal(literal) => {
                    set_attribute(attributes, attribute, literal.as_text());
                }
                Operand::Static(value) => self.bind(
                    target.clone(),
                    BindKind::AttributeOnce {
                        name: attribute.to_string(),
                        value,
                    },
                ),
                Operand::Reactive(getter) => self.bind(
                    target.clone(),
                    BindKind::Attribute {
                        name: attribute.to_string(),
                        getter,
                    },
                ),
            },
        }
    }

    /// One style argument, folded into the element's generated class when
    /// its value is known and bound through the CSSOM when it is not.
    ///
    /// The two paths are not the same rule and never were. A folded
    /// declaration is **printed** into `styles.css`, so its value is not
    /// confined to the declaration it sits in: `weight is "bold; } body {
    /// display: none } x {"` is a rule for `body`, which is a defacement
    /// of the whole page. A bound one reaches `style.setProperty`, which
    /// parses one declaration and drops it whole if it does not parse, so
    /// the worst a value can do there is nothing.
    ///
    /// What the grammar adds on top of that older argument is *meaning*.
    /// The character allowlist says a value cannot end its declaration; it
    /// cannot say that `color is "reddish"` is not a colour, and a
    /// declaration a browser drops on the floor is a program that renders
    /// wrongly with no diagnostic anywhere. So the folded path asks the
    /// argument's own grammar, and the message names what the argument
    /// admits rather than what this value was not.
    fn style_argument(
        &mut self,
        name: &str,
        argument: elements::StyleArgument,
        operand: Operand,
        element: &HirElement,
        target: &Address,
        declarations: &mut Vec<Declaration>,
    ) {
        // A URL is written down, or it is not written at all.
        //
        // Every other grammar can be bound: `setProperty` parses one
        // declaration, so a runtime value that is not a colour sets
        // nothing at all and the worst case is a rendering bug the program
        // can see. A URL is not like that. It is printed *inside*
        // `url(…)`, and no runtime check keeps a value between those
        // parentheses. `safeUrl` rules on schemes, which is a different
        // question, so a bound backdrop would be a request to a host the
        // program never named. Refusing it costs a feature nobody asked
        // for; admitting it would cost the property this grammar exists
        // to hold.
        if argument.grammar == style::Grammar::Url && !matches!(operand, Operand::Literal(_)) {
            self.emitter.error(
                format!(
                    "`{name}` must be written down. A URL here is printed into `url(…)` in \
                     `styles.css`, and a value that exists only at run time cannot be checked \
                     against the delimiters it would sit between."
                ),
                element.span,
            );
            return;
        }
        // A conditioned declaration is written down, or not at all.
        //
        // There is no element whose `:hover` is an element, and no
        // `setProperty` that takes a media query, so a conditioned
        // declaration exists only in the printed sheet. Binding one would
        // mean emitting a rule at run time, which means a stylesheet the
        // program writes into at run time, which is the CSS-injection
        // surface this whole design closes.
        if argument.condition != style::Condition::Always && !matches!(operand, Operand::Literal(_))
        {
            self.emitter.error(
                format!(
                    "`{name}` must be written down. It becomes a rule of its own in \
                     `styles.css`, a `:hover` or a query, and there is no element whose \
                     `:hover` is an element, so a value that exists only at run time has \
                     nowhere to go."
                ),
                element.span,
            );
            return;
        }
        // A value the compiler *translates* is written down, or not at all.
        //
        // `decoration is "struck"` becomes `line-through` and `opacity is
        // 50` becomes `0.5`, and both translations happen where the class
        // is folded. A bound value reaches `setProperty` as the program
        // wrote it, so a signal holding `"struck"` would set nothing and
        // one holding `"line-through"` would work: a language where the
        // dynamic spelling is CSS's and the static spelling is not.
        // `opacity` fails more quietly still: 50 is out of range, the
        // browser clamps it to 1, and the element is opaque rather than
        // half.
        //
        // What a program writes instead is `if`, which the view already
        // has: two elements, one styled and one not, and the signal graph
        // swaps them. `examples/todo.zd` is written that way.
        if translated_when_folded(argument.grammar) && !matches!(operand, Operand::Literal(_)) {
            self.emitter.error(
                format!(
                    "`{name}` must be written down. Its value is translated into CSS where the \
                     class is folded, so `struck` becomes `line-through` and `50` becomes \
                     `0.5`, and a value that exists only at run time would reach the browser \
                     untranslated. Write `if` in the view and style the two branches \
                     differently."
                ),
                element.span,
            );
            return;
        }
        match operand {
            Operand::Literal(literal) => {
                let written = literal.as_text();
                let Some(value) = style::value(argument.grammar, &written) else {
                    self.emitter.error(
                        format!(
                            "`{}` may not be styled `{name} is \"{written}\"`. A `{name}` is {} \
                             (spec §16.3.11). The set is closed rather than escaped because a \
                             style value is folded into a rule in `styles.css`, which prints it: \
                             anything else would end that rule and begin another for a selector \
                             nothing here wrote.",
                            element.name,
                            style::expectation(argument.grammar)
                        ),
                        element.span,
                    );
                    return;
                };
                let value = match argument.suffix {
                    Some(suffix) => format!("{value} {suffix}"),
                    None => value,
                };
                declarations.push(Declaration {
                    condition: argument.condition,
                    property: argument.property.to_string(),
                    value,
                });
            }
            // A value, not a getter: `static` is inlined as a literal, so
            // `({value})() + 'px'` called a number. It cannot change
            // either, so it is set once at clone time rather than inside
            // an effect, the same shape `Named::Attribute` gives the same
            // operand.
            Operand::Static(value) => {
                let value = self.style_expression(argument, value, false);
                self.bind(
                    target.clone(),
                    BindKind::StyleOnce {
                        property: argument.property.to_string(),
                        value,
                    },
                );
            }
            Operand::Reactive(getter) => {
                let getter = self.style_expression(argument, getter, true);
                self.bind(
                    target.clone(),
                    BindKind::Style {
                        property: argument.property.to_string(),
                        getter,
                    },
                );
            }
        }
    }

    /// The JavaScript that turns a runtime value into the declaration's
    /// value, for the grammars whose written form is not what CSS wants.
    ///
    /// Only the unit is added here. Nothing checks at runtime that a
    /// reactive `color` names a colour, and nothing needs to: `setProperty`
    /// parses one declaration, so a value that is not a colour sets
    /// nothing at all. That is a rendering bug the program can see, not a
    /// rule for a selector it did not write.
    fn style_expression(
        &mut self,
        argument: elements::StyleArgument,
        source: String,
        reactive: bool,
    ) -> String {
        let unit = match argument.grammar {
            style::Grammar::Lengths => "'px'",
            // A bound `rotate is angle` has to carry its unit for the same
            // reason a length does: `setProperty` parses one declaration,
            // and `rotate: 30` is not one — CSS drops it and the element
            // simply does not turn, with nothing said anywhere.
            style::Grammar::Angle => "'deg'",
            // unreached for `Url`: a bound one was refused above.
            style::Grammar::Number
            | style::Grammar::Whole
            | style::Grammar::Percent
            | style::Grammar::Colour
            | style::Grammar::Url
            | style::Grammar::Keyword(_)
            | style::Grammar::Free => return source,
        };
        // The argument's own suffix, which a *static* declaration gets from
        // `style::value` and a bound one was losing.
        //
        // `border` is the one argument that carries one, and it carries it
        // because a width alone draws nothing: `border-style` defaults to
        // `none`, so `border: 26px` is a declaration CSS accepts and does
        // not paint. `border is 1` folded at build time became `border: 1px
        // solid` and worked; `border is ring.width` became `border: 26px`
        // and drew an invisible box, with nothing anywhere saying why.
        let tail = match argument.suffix {
            Some(suffix) => format!(" + {}", js::string(&format!(" {suffix}"))),
            None => String::new(),
        };
        if reactive {
            format!("() => ({source})() + {unit}{tail}")
        } else {
            format!("({source}) + {unit}{tail}")
        }
    }

    /// A `class` that is not a literal, emitted once the element's class
    /// list is settled.
    ///
    /// `js::string`, never `'{base} '`. The base is the element's own
    /// classes joined, and a program can put its own text among them, so
    /// interpolating it raw into a JavaScript string literal let a
    /// source-level `class is "a'+alert(1)+'b"` close the quote and write
    /// expressions into the emitted module.
    ///
    /// The two operands are spelled apart because an `Operand::Static` is
    /// a **value** and not a getter (§14C.3b): a `static` signal is
    /// inlined as the literal the build host printed, so calling it is
    /// calling a string. One assignment at clone time is also all it can
    /// ever need, since nothing about it changes.
    fn class_binding(&mut self, operand: Operand, classes: &[String], target: &Address) {
        let base = js::string(&format!("{} ", classes.join(" ")));
        let kind = match operand {
            // unreached: a literal `class` joined the class list instead of
            // being held, so it never reaches this.
            Operand::Literal(literal) => BindKind::AttributeOnce {
                name: "class".to_string(),
                value: format!("{base} + {}", js::string(&literal.as_text())),
            },
            Operand::Static(value) => BindKind::AttributeOnce {
                name: "class".to_string(),
                value: format!("{base} + ({value})"),
            },
            Operand::Reactive(getter) => BindKind::Attribute {
                name: "class".to_string(),
                getter: format!("() => {base} + ({getter})()"),
            },
        };
        self.bind(target.clone(), kind);
    }

    /// An attribute that carries a URL.
    ///
    /// A literal is checked here and refused outright, which is the whole
    /// check when the destination is written in the source. A computed one
    /// cannot be: `setAttribute('href', …)` parses no HTML, so §16.3.5's
    /// escaping argument holds, but it happily accepts `javascript:`, which
    /// that argument never covered. So the getter is wrapped in `safeUrl`
    /// and the filter runs where the value actually arrives.
    /// State that a link goes to the document it is written in (#142).
    ///
    /// `aria-current="page"` is what a screen reader announces the current
    /// item of a navigation with, and without it a list of links has no
    /// current item for anybody not looking at it. **A framework cannot
    /// add this without shipping a comparison to run in the browser**: it
    /// has to know the document's own URL and every link's destination at
    /// the same moment. Here both are compile-time — §14G.2's bijection
    /// renders a route value to its URL, and the address fold emits one
    /// document per URL — so the attribute is written into the markup and
    /// costs the reader nothing.
    ///
    /// Only a destination the compiler settled to a literal. A URL a
    /// program computes is not known here, and guessing at run time would
    /// be the shipped comparison this avoids.
    fn mark_current_page(&mut self, operand: &Operand, attributes: &mut Vec<(String, String)>) {
        let (Some(url), Operand::Literal(literal)) = (self.page_url, operand) else {
            return;
        };
        if literal.as_text() == url {
            set_attribute(attributes, "aria-current", "page".to_string());
        }
    }

    fn url_attribute(
        &mut self,
        attribute: &'static str,
        operand: Operand,
        element: &HirElement,
        target: &Address,
        attributes: &mut Vec<(String, String)>,
    ) {
        match operand {
            Operand::Literal(literal) => {
                let url = literal.as_text();
                if elements::url_is_permitted(&url) {
                    set_attribute(attributes, attribute, url);
                    return;
                }
                // unreached: `zdc-graph`'s flow pass reports this first, as
                // E-URL-01, and a program it refuses is one codegen never
                // runs on — `Inputs` cannot be built without a clearance.
                // Kept because the two rules read the same list
                // (`zdc_hir::URL_SCHEMES`) and this is the emission site;
                // if the flow pass ever stopped ranging over a position
                // this one covers, the bytes would still be filtered.
                self.emitter.error(
                    format!(
                        "`{}` may not point at `{url}`. A URL here is either relative or one of \
                         {}; anything else is script execution behind a click.",
                        element.name,
                        english_list(elements::URL_SCHEMES)
                    ),
                    element.span,
                );
            }
            Operand::Static(value) => {
                self.used_safe_url();
                self.bind(
                    target.clone(),
                    BindKind::AttributeOnce {
                        name: attribute.to_string(),
                        value: format!("safeUrl({value})"),
                    },
                );
            }
            Operand::Reactive(getter) => {
                self.used_safe_url();
                self.bind(
                    target.clone(),
                    BindKind::Attribute {
                        name: attribute.to_string(),
                        getter: format!("() => safeUrl(({getter})())"),
                    },
                );
            }
        }
    }

    /// An ARIA state attribute, whose value is the **word** `true` or the
    /// word `false`.
    ///
    /// The bound arm is the whole reason this is not a
    /// [`Named::Attribute`]. `dom.js`'s `setAttribute` implements HTML's
    /// boolean attributes — `false` removes the attribute, `true` sets it
    /// to the empty string — which is right for `hidden` and wrong for
    /// every `aria-*` state, where the value is an enumerated string. A
    /// tab bound to `selected is index is chosen` would carry
    /// `aria-selected=""` when chosen and *no attribute at all* when not,
    /// and a tablist where no tab says `false` is announced as one with
    /// nothing selected. Nothing warns; the page renders; the reader is
    /// told the wrong thing.
    ///
    /// So the getter is wrapped where it is bound, exactly as
    /// `style_expression` appends a unit to a bound `rotate` for the same
    /// class of reason: a value that is correct in the program and wrong
    /// by the time the browser reads it.
    ///
    /// The other two operands need no wrapper and get none. A literal is
    /// baked as `true`/`false` because `Literal::as_text` already prints a
    /// `Truth` that way, and an `Operand::Static` goes through
    /// `AttributeOnce`, which is `setAttribute(name, String(value))` — the
    /// same two words by a different route.
    fn state_attribute(
        &mut self,
        attribute: &'static str,
        operand: Operand,
        target: &Address,
        attributes: &mut Vec<(String, String)>,
    ) {
        match operand {
            Operand::Literal(literal) => {
                set_attribute(attributes, attribute, literal.as_text());
            }
            Operand::Static(value) => self.bind(
                target.clone(),
                BindKind::AttributeOnce {
                    name: attribute.to_string(),
                    value,
                },
            ),
            Operand::Reactive(getter) => self.bind(
                target.clone(),
                BindKind::Attribute {
                    name: attribute.to_string(),
                    getter: format!("() => String(({getter})())"),
                },
            ),
        }
    }

    /// An ARIA attribute admitting one word from a closed set.
    ///
    /// Written down, or not written at all. There is no run-time check
    /// that could stand in for this one: an ARIA token outside the set is
    /// not ignored, it is *mapped* — every unrecognised value of
    /// `aria-current` means `true` — so a signal holding `"pge"` announces
    /// the wrong kind of current rather than none, in a program that
    /// compiled clean. Refusing here is the same rule the style vocabulary
    /// applies to a bound `decoration`, and the same answer applies: write
    /// `if` in the view, which is how `widgets/breadcrumbs.zd` says which
    /// step is the page you are on.
    fn word_attribute(
        &mut self,
        name: &str,
        attribute: &'static str,
        words: &'static [&'static str],
        operand: Operand,
        element: &HirElement,
        attributes: &mut Vec<(String, String)>,
    ) {
        let Operand::Literal(literal) = operand else {
            self.emitter.error(
                format!(
                    "`{name}` must be written down. It admits one of {}, and a browser maps \
                     any other word onto a different meaning rather than ignoring it. Write \
                     `if` in the view and give the branches different words.",
                    english_list(words)
                ),
                element.span,
            );
            return;
        };
        let written = literal.as_text();
        if !words.contains(&written.as_str()) {
            self.emitter.error(
                format!(
                    "`{}` may not be given `{name} is \"{written}\"`. A `{name}` is one of {}, \
                     and a browser maps any other word onto a meaning nobody wrote.",
                    element.name,
                    english_list(words)
                ),
                element.span,
            );
            return;
        }
        set_attribute(attributes, attribute, written);
    }

    fn used_safe_url(&mut self) {
        self.emitter.used.dom.insert("safeUrl");
    }

    /// `Item` outside a list is an orphaned `<li>`, which a screen reader
    /// reads as ordinary text. The nesting the element's meaning requires
    /// is checked rather than assumed.
    fn check_placement(&mut self, element: &HirElement, shape: &elements::Shape) {
        if shape.only_inside.is_empty() {
            return;
        }
        let inside_one = self
            .parent
            .is_some_and(|parent| shape.only_inside.contains(&parent));
        if inside_one {
            return;
        }
        self.emitter.error(
            format!(
                "`{}` must be written directly inside {}.",
                element.name,
                english_list(shape.only_inside)
            ),
            element.span,
        );
    }

    /// The check is over what ends up a *DOM* child, not over what is
    /// written as a HIR child. `each`, `if`, `when` and a component's own
    /// scope place their contents directly in the parent — there is no
    /// element between them and it — so a `Column` under `List / each` is
    /// a `<div>` inside a `<ul>` exactly as a bare one would be. Checking
    /// only the direct `HirNode::Element` children let every one of those
    /// through.
    fn check_only_children(
        &mut self,
        element: &HirElement,
        shape: &elements::Shape,
        children: &[HirNode],
    ) {
        if shape.only_phrasing_children {
            let mut placed = Vec::new();
            placed_elements(children, &mut placed);
            for child in placed {
                let child_shape = elements::shape(&child.name);
                if child_shape.is_none_or(|s| elements::is_phrasing_tag(s.tag)) {
                    continue;
                }
                // Named as a content model rather than as a list, because
                // the list is the wrong thing to memorise: the rule is the
                // HTML parser's, it applies to every block element there
                // is, and the repair is to move the block outside the
                // paragraph rather than to reach for a different one.
                self.emitter.error(
                    format!(
                        "`{}` takes only text and inline elements; `{}` is a block element. \
                         The HTML parser closes a paragraph before a block, so this would \
                         render outside the `{}` rather than inside it.",
                        element.name, child.name, element.name
                    ),
                    child.span,
                );
            }
        }

        if shape.only_children.is_empty() {
            return;
        }
        let mut placed = Vec::new();
        placed_elements(children, &mut placed);
        for child in placed {
            if shape.only_children.contains(&child.name.as_str()) {
                continue;
            }
            self.emitter.error(
                format!(
                    "`{}` takes only {}; `{}` is not one.",
                    element.name,
                    english_list(shape.only_children),
                    child.name
                ),
                child.span,
            );
        }
    }

    /// A masked value may be typed and nowhere else.
    ///
    /// `PasswordInput`'s doc comment in `elements.rs` states the secrecy
    /// decision this enforces; the short form is that the lattice's
    /// `Secret` cannot label a value that is born in the browser, so the
    /// rule the lattice would have given is written here instead, over the
    /// one place a view can leak: **the signal a `PasswordInput` binds may
    /// appear in the view as that field's own binding and nowhere else.**
    ///
    /// That covers being shown, being put in a URL-bearing attribute, and
    /// being mirrored into a second, unmasked field, without naming any of
    /// the three: the rule is about the value's one legitimate use rather
    /// than about a list of sinks that would have to grow with the
    /// vocabulary.
    ///
    /// A handler sending it somewhere is untouched, and deliberately. That
    /// is what a password is for, and §14B.5's placement rule and the flow
    /// pass already range over that path.
    fn check_masked(&mut self, element: &HirElement) {
        if self.masked.is_empty() {
            return;
        }
        let binding = if element.name == "PasswordInput" {
            zdc_hir::destination_of(element).or_else(|| leading_positional(element))
        } else {
            None
        };
        for arg in &element.args {
            let expr = crate::analysis::arg_expr(arg);
            if Some(expr) == binding {
                continue;
            }
            let mut referenced = Vec::new();
            crate::analysis::expr_references(self.emitter.hir, expr, &mut referenced);
            let Some(def) = referenced.iter().find(|def| self.masked.contains(def)) else {
                continue;
            };
            self.emitter.error(
                format!(
                    "`{}` is what a `PasswordInput` binds, so it can be typed and nothing else. \
                     This would put it in a `{}`, where it is either shown to whoever is looking \
                     at the screen or handed to whichever host the value names. Send it from a \
                     handler instead.",
                    self.emitter.hir.defs[*def].name, element.name
                ),
                element.span,
            );
        }
    }

    /// The child that supplies an element's own accessible name, checked
    /// where the element is written rather than trusted.
    ///
    /// `Fieldset` needs a `Legend` and `Details` needs a `Summary`, and
    /// both render *worse* without one than plain markup would: the group
    /// is announced with no subject, and the disclosure is labelled with
    /// whatever word the browser chose. So the name is asked for, exactly
    /// as `Image` asks for `alt`.
    ///
    /// Checked over what ends up a **DOM** child, so a `Legend` written
    /// inside an `if` still counts as the first one: `if`, `each`, `when`
    /// and a component's scope place their contents directly in the
    /// parent. That a conditional legend may be absent at run time is a
    /// hole this check cannot close, and a check that refused the
    /// construct outright would refuse a program that is right.
    fn check_leading_child(
        &mut self,
        element: &HirElement,
        shape: &elements::Shape,
        children: &[HirNode],
    ) {
        let Some(required) = shape.leading_child else {
            return;
        };
        let mut placed = Vec::new();
        placed_elements(children, &mut placed);
        if placed.first().is_some_and(|first| first.name == required) {
            return;
        }
        self.emitter.error(
            format!(
                "`{}` begins with `{required}`, which is where its name comes from. Write \
                 `{required} \"…\"` as its first child.",
                element.name
            ),
            element.span,
        );
    }

    /// A text node for a slot: baked when it is a non-empty literal,
    /// otherwise a deliberate single space for a binding to write into.
    ///
    /// The space matters. `<span></span>` has no child at all and
    /// `firstChild` is `null`; the parser materialises a text node only
    /// when there is something between the tags. The binding's effect runs
    /// synchronously at construction, before `mount` puts the tree in the
    /// document, so the space is never painted.
    fn text_child(&mut self, operand: Operand, children: &mut Vec<Tpl>, parent: &Address) {
        let index = children.len();
        let mut target = parent.clone();
        target.push(index);

        match operand {
            Operand::Literal(literal) if !literal.as_text().is_empty() => {
                children.push(Tpl::Text(literal.as_text()));
            }
            Operand::Literal(literal) => {
                children.push(Tpl::Text(" ".to_string()));
                self.bind(target, BindKind::TextOnce(literal.as_js()));
            }
            Operand::Static(value) => {
                children.push(Tpl::Text(" ".to_string()));
                self.bind(target, BindKind::TextOnce(value));
            }
            Operand::Reactive(getter) => {
                children.push(Tpl::Text(" ".to_string()));
                self.bind(target, BindKind::Text(getter));
            }
        }
    }

    /// `Prose post.body`: the document becomes the element's content, by
    /// being parsed.
    ///
    /// The value is written onto the element itself rather than into a
    /// child text node, because it is not text — there is no node to write
    /// into until the parser has made some. So no placeholder child is
    /// pushed and the address is the element's own.
    ///
    /// A literal is treated exactly as a computed value: it still goes
    /// through `markup()` at construction rather than being interpolated
    /// into the template string. That keeps §16.3.5's rule — *only
    /// compile-time string literals of the program are interpolated into
    /// `innerHTML`* — true as written, since a rendered document is a
    /// literal of a **file**, not of the program.
    fn markup_child(&mut self, operand: Operand, target: &Address) {
        let target = target.clone();
        match operand {
            Operand::Literal(literal) => self.bind(target, BindKind::MarkupOnce(literal.as_js())),
            Operand::Static(value) => self.bind(target, BindKind::MarkupOnce(value)),
            Operand::Reactive(getter) => self.bind(target, BindKind::Markup(getter)),
        }
    }

    /// `Input name` and `Checkbox done`: one attribute binding plus one
    /// listener, both on the cloned node.
    /// `Select showing`: one option per arm of the choice `showing` holds,
    /// written by the compiler, plus a two-way binding over the tag.
    ///
    /// The options come from the *declaration* rather than from the
    /// program, which is the whole of what this element buys: the set a
    /// reader can pick from is the set the type admits, and a variant
    /// added to the choice appears here without anything else being
    /// edited. There is no second list to drift.
    ///
    /// A variant that carries fields is refused. An option's value is one
    /// string, so there is nowhere for a payload to come from, and
    /// inventing one would be inventing data.
    fn choice(
        &mut self,
        element: &HirElement,
        expr: ExprId,
        target: &Address,
        children: &mut Vec<Tpl>,
    ) {
        let Some((getter, setter)) = self.bound_signal(element, expr) else {
            return;
        };
        let Some(variants) = self.choice_variants(element, expr) else {
            return;
        };
        for offered in &variants {
            children.push(Tpl::Element {
                tag: "option",
                attributes: vec![("value".to_string(), offered.tag.clone())],
                children: vec![Tpl::Text(offered.shown.clone())],
            });
        }
        // The tag, not the variant: an option's value is a string, and
        // `.tag` is the string the runtime's own `variant` puts there.
        self.bind(
            target.clone(),
            BindKind::Attribute {
                name: "value".to_string(),
                getter: format!("() => ({getter})().tag"),
            },
        );
        // Built with the runtime's `variant`, for the reason the preamble
        // helpers use it: `when` dispatches on the shape `variant`
        // produces, and a second place that knows that shape is a second
        // place that can get it wrong.
        self.emitter.used.dom.insert("variant");
        self.bind(
            target.clone(),
            BindKind::Listener {
                event: "change".to_string(),
                handler: format!(
                    "({TWO_WAY_PARAMETER}) => \
                     {setter}(variant({TWO_WAY_PARAMETER}.target.value))"
                ),
            },
        );
    }

    /// `Radio showing, option is All`: one button of a group.
    ///
    /// The group is the *signal*, named by the local the emitter gave it,
    /// which is unique per definition. Every radio bound to one signal
    /// therefore shares a `name`, the browser clears the others when one
    /// is picked, and nothing in the program maintains the invariant that
    /// exactly one is set: the signal holds one variant because a variant
    /// is one thing.
    fn radio(
        &mut self,
        element: &HirElement,
        expr: ExprId,
        target: &Address,
        attributes: &mut Vec<(String, String)>,
    ) {
        let Some((getter, setter)) = self.bound_signal(element, expr) else {
            return;
        };
        let Some(variants) = self.choice_variants(element, expr) else {
            return;
        };
        let Some(option) = named_argument_of(element, "option") else {
            // unreached: `zdc-types` reports the missing required argument
            // first, in its own words.
            self.emitter.error(
                format!("`{}` needs `option is …`.", element.name),
                element.span,
            );
            return;
        };
        let HirExprKind::Ref(Res::Variant { choice, index }) = self.emitter.hir.exprs[option].kind
        else {
            self.emitter.error(
                format!(
                    "`{}` needs `option is …` to name one arm of the choice it binds, written \
                     down. It becomes this button's value in the markup, so it cannot be \
                     computed.",
                    element.name
                ),
                self.emitter.hir.exprs[option].span,
            );
            return;
        };
        let DefKind::Choice(declared) = self.emitter.hir.defs[choice].kind.clone() else {
            // unreached: `zdc-resolve` builds `Res::Variant` from a choice
            // definition, so the definition it names is one.
            self.emitter.error(
                format!(
                    "`{}` names a variant of something that is not a choice.",
                    element.name
                ),
                element.span,
            );
            return;
        };
        let Some(tag) = declared
            .variants
            .get(index as usize)
            .map(|v| v.name.clone())
        else {
            // unreached: `zdc-resolve` produced the index from the same
            // list this reads.
            self.emitter.error(
                format!("`{}` names an arm that is not in the choice.", element.name),
                element.span,
            );
            return;
        };
        if !variants.iter().any(|offered| offered.tag == tag) {
            // unreached: `zdc-types` reports this first, in its own words.
            // `option` and the bound signal are both typed, and a variant
            // of one choice is not a value of another, so a mismatch is a
            // type error before emission runs. Kept because the tag this
            // writes into the markup comes from the *declaration* rather
            // than from the type, and the two reaching different answers
            // would be a button that can never be chosen.
            self.emitter.error(
                format!(
                    "`{tag}` is not an arm of the choice `{}` binds, so this button could never \
                     be the one that is chosen.",
                    element.name
                ),
                element.span,
            );
            return;
        }

        // The group, and the button's own value. Both are compile-time
        // constants, so both are markup rather than bindings.
        set_attribute(attributes, "name", getter.clone());
        set_attribute(attributes, "value", tag.clone());

        self.bind(
            target.clone(),
            BindKind::Attribute {
                name: "checked".to_string(),
                getter: format!("() => ({getter})().tag === {}", js::string(&tag)),
            },
        );
        self.emitter.used.dom.insert("variant");
        self.bind(
            target.clone(),
            BindKind::Listener {
                event: "change".to_string(),
                handler: format!("() => {setter}(variant({}))", js::string(&tag)),
            },
        );
    }

    /// The arms of the choice a `Select`'s signal is declared to hold.
    ///
    /// Read off the *declaration* rather than off a type, which is what
    /// keeps this file's rule that nothing in it consults `zdc-types`:
    /// a signal's `ty` is the `TypeExpr` the program wrote, and a name in
    /// it is a definition this pass can look up.
    fn choice_variants(&mut self, element: &HirElement, expr: ExprId) -> Option<Vec<Offered>> {
        let HirExprKind::Ref(Res::Def(def)) = self.emitter.hir.exprs[expr].kind else {
            // unreached: `bound_signal` has already refused anything that
            // is not a `state` name.
            return None;
        };
        let DefKind::Signal(signal) = &self.emitter.hir.defs[def].kind else {
            // unreached: `bound_signal` refuses this first.
            return None;
        };
        let zdc_ast::TypeExpr::Named(named) = &signal.ty else {
            // unreached: `zdc-types` reports this first, in its own words.
            // A `List of …`, a `Map`, an `Option` and a `Remote` are all
            // types the checker has and none of them is `Type::Named`, so
            // `Bound::Variant` refuses them before emission runs.
            self.emitter.error(
                format!(
                    "`{}` binds one variant of a `choice`, and `{}` is not declared as one.",
                    element.name, self.emitter.hir.defs[def].name
                ),
                element.span,
            );
            return None;
        };
        let name = named.text.clone();
        let found = self
            .emitter
            .hir
            .defs
            .iter()
            .find(|(_, other)| other.name == name && matches!(other.kind, DefKind::Choice(_)))
            .map(|(_, other)| other.kind.clone());
        let Some(DefKind::Choice(choice)) = found else {
            self.emitter.error(
                format!(
                    "`{}` binds one variant of a `choice`, and `{name}` is not a `choice` this \
                     file declares or imports.",
                    element.name
                ),
                element.span,
            );
            return None;
        };
        for variant in &choice.variants {
            if !variant.fields.is_empty() {
                self.emitter.error(
                    format!(
                        "`{name}`'s `{}` carries fields, and an option's value is one word, so \
                         there is nowhere for them to come from. A `{}` offers a `choice` whose \
                         arms are all bare.",
                        variant.name, element.name
                    ),
                    element.span,
                );
                return None;
            }
        }
        if choice.variants.is_empty() {
            // unreached: `zdc-resolve` refuses a `choice` with no arms.
            self.emitter.error(
                format!("`{name}` has no arms, so there is nothing to offer."),
                element.span,
            );
            return None;
        }
        Some(
            choice
                .variants
                .iter()
                .map(|variant| Offered {
                    // The tag is what the runtime's `variant` round-trips,
                    // so it is the name and never the label. A label that
                    // reached the value would make two variants with the
                    // same label indistinguishable on the way back.
                    tag: variant.name.clone(),
                    shown: variant
                        .label
                        .clone()
                        .unwrap_or_else(|| variant.name.to_string()),
                })
                .collect(),
        )
    }

    fn two_way(
        &mut self,
        element: &HirElement,
        expr: ExprId,
        attribute: &str,
        bound: &str,
        target: &Address,
    ) {
        let Some((getter, setter)) = self.bound_signal(element, expr) else {
            return;
        };
        // The sugar is a handler with a payload, written by the compiler.
        // Both the event and the accessor come from the shared event table,
        // so `Input name` and a hand-written `on input with e / set name to
        // e.value` cannot disagree about what `value` means.
        let (Some(event), Some(handler)) = (
            crate::events::two_way_event(bound),
            crate::events::two_way_listener(bound, TWO_WAY_PARAMETER, &setter),
        ) else {
            // unreached: An internal guard. `two_way` is called with the
            // three keys the event table answers.
            self.emitter.error(
                format!("`{}` has no two-way binding.", element.name),
                element.span,
            );
            return;
        };
        self.bind(
            target.clone(),
            BindKind::Attribute {
                name: attribute.to_string(),
                getter,
            },
        );
        self.bind(
            target.clone(),
            BindKind::Listener {
                event: event.to_string(),
                handler,
            },
        );
    }

    /// `NumberInput count` and `DateInput arriving`: two-way to an
    /// `Option` of a number, through `valueAsNumber` in both directions.
    ///
    /// Structurally `two_way` with the attribute binding replaced. It
    /// shares `bound_signal`, so §14B.5's rules about which signals a
    /// control may write are the same rules, and it shares the event
    /// table, so what the listener reads is the same property `Slider`
    /// reads. What differs is written down in [`BindKind::NumberField`]
    /// and in `intrinsics.rs`.
    fn optional_number(&mut self, element: &HirElement, expr: ExprId, target: &Address) {
        let Some((getter, setter)) = self.bound_signal(element, expr) else {
            return;
        };
        let Some(handler) =
            crate::events::two_way_listener(OPTIONAL_NUMBER_KEY, TWO_WAY_PARAMETER, &setter)
        else {
            // unreached: An internal guard. The key is one the event
            // table answers.
            self.emitter.error(
                format!("`{}` has no two-way binding.", element.name),
                element.span,
            );
            return;
        };
        self.emitter.use_helper("$optionalNumber");
        self.emitter.use_helper("$numberField");
        self.bind(target.clone(), BindKind::NumberField(getter));
        self.bind(
            target.clone(),
            BindKind::Listener {
                event: "input".to_string(),
                handler,
            },
        );
    }

    /// `Dialog confirming`: the signal that says whether the modal is
    /// showing, in both directions.
    ///
    /// It shares `bound_signal` with every other two-way element, so
    /// §14B.5's rule about which signals an interaction may write is the
    /// same rule — a `durable` signal here would make dismissing a dialog
    /// a network write, which is what that rule exists to refuse. What
    /// differs from `two_way` is that neither half is an attribute: the
    /// read is `showModal()`/`close()` and the write is a `close` event,
    /// and both are inside `$modal` because they have to agree about what
    /// the DOM is currently doing. `Slot::Open` argues it.
    fn modal(&mut self, element: &HirElement, expr: ExprId, target: &Address) {
        let Some((getter, setter)) = self.bound_signal(element, expr) else {
            return;
        };
        self.emitter.use_helper("$modal");
        self.bind(target.clone(), BindKind::Modal { getter, setter });
    }

    /// The `[read, write]` pair of the signal a two-way slot binds, with
    /// §14B.5's rules already applied.
    ///
    /// Shared by every two-way element, so that a `Select` and an `Input`
    /// cannot come to disagree about which signals a control may write.
    fn bound_signal(&mut self, element: &HirElement, expr: ExprId) -> Option<(String, String)> {
        let span = self.emitter.hir.exprs[expr].span;
        let HirExprKind::Ref(Res::Def(def)) = self.emitter.hir.exprs[expr].kind else {
            // unreached: `zdc-types` reports this first, in its own words.
            self.emitter.error(
                format!(
                    "`{}` binds two-way, so it needs a `state` name rather than an expression \
                     (spec §14B.5).",
                    element.name
                ),
                span,
            );
            return None;
        };
        let DefKind::Signal(signal) = &self.emitter.hir.defs[def].kind else {
            // unreached: `zdc-types` reports this first, in its own words.
            self.emitter.error(
                format!("`{}` binds two-way and needs `state`.", element.name),
                span,
            );
            return None;
        };
        let placement = signal.placement;
        let is_source = signal.is_source;
        let declared = self.emitter.hir.defs[def].name.clone();

        if !is_source {
            // unreached: `zdc-types` reports this first, in its own words.
            self.emitter.error(
                format!(
                    "`{declared}` is declared with `from`, so the compiler recomputes it. A \
                     two-way binding needs a `starting` signal."
                ),
                span,
            );
            return None;
        }
        // The same classification `zdc-types`' `check_two_way` makes, and
        // for the same reason: a keystroke must not silently become a
        // *request*. A `remembered` write is local and synchronous, so it
        // binds exactly as a `client` cell does — and the setter emitted
        // for one already writes the store, so nothing here changes.
        let binds_without_a_request = matches!(
            zdc_types::SignalPlacement::from_ast(placement),
            zdc_types::SignalPlacement::Client | zdc_types::SignalPlacement::Remembered
        );
        if !binds_without_a_request {
            // unreached: `zdc-types` reports this first, in its own words.
            self.emitter.error(
                format!(
                    "`{declared}` is {placement:?}-placed, and a keystroke must not silently \
                     become a network write (spec §14B.5)."
                ),
                span,
            );
            return None;
        }

        let getter = self.emitter.names.def(def).to_string();
        let Some(setter) = self.emitter.names.setter(def).map(str::to_string) else {
            // unreached: An internal guard. The arms above leave only a
            // `starting` client signal, which is emitted with its setter.
            self.emitter.error(
                format!("`{declared}` is bound two-way but was given no setter."),
                element.span,
            );
            return None;
        };
        Some((getter, setter))
    }

    fn listener(
        &mut self,
        element: &HirElement,
        slot: Slot,
        handler: &HirHandler,
        target: &Address,
    ) {
        if (matches!(slot, Slot::Value | Slot::Level | Slot::OptionalLevel)
            && handler.event == "input")
            || (slot == Slot::Checked && handler.event == "change")
        {
            self.emitter.error(
                format!(
                    "`{}` already wires `on {}` as its two-way binding, so a second handler for it \
                     would fight the built-in one (spec §16.3.6).",
                    element.name, handler.event
                ),
                handler.span,
            );
            return;
        }
        let mut source = self.handler_source(handler);
        // A submit handler runs *instead of* the browser's own submission,
        // never before it. Without this the handler runs and the page then
        // navigates anyway, which is the same loss one frame later and is
        // the reason `Form` requires the handler at all.
        //
        // `$e` is hygienic against every name a program can spell: `$` is
        // in neither XID_Start nor XID_Continue. The handler is called with
        // the event whether or not it bound one, because an arrow that
        // declares no parameter ignores what it is passed.
        if element.name == "Form" && handler.event == "submit" {
            source = format!("($e) => {{ $e.preventDefault(); ({source})($e); }}");
        }
        self.bind(
            target.clone(),
            BindKind::Listener {
                event: handler.event.clone(),
                handler: source,
            },
        );
    }

    /// A handler's body. No `batch(...)` wrapper is ever emitted: `on()`
    /// already wraps every listener in one (spec §16.3.7).
    ///
    /// A handler that bound the event takes it as its parameter, under the
    /// name the program gave it. A handler that did not takes none, which
    /// is the emission every existing program already has.
    ///
    /// # Why a handler that writes across a boundary is `async`
    ///
    /// A cross-region write is a network call. Emitted as a bare
    /// expression statement it becomes fire-and-forget, and three of them
    /// in one handler produce three requests that:
    ///
    /// - can land in any order, so `set x` then `add 1 to x` is a race;
    /// - have nowhere to report a failure, since the promise is discarded
    ///   and an unhandled rejection is a console entry at best;
    /// - half-apply invisibly, because the second failing does not stop
    ///   the third.
    ///
    /// Awaiting them in order fixes the first two outright and turns the
    /// third from silent into reported-and-stopped.
    ///
    /// # The handler is the transaction
    ///
    /// Awaiting the writes fixed ordering and reporting and left the third
    /// problem standing: three writes were three requests and three store
    /// operations, so a failure on the second left the first committed
    /// with nothing to roll it back. For a vote spread over eight keys
    /// that is corrupt data rather than a failed request.
    ///
    /// So the writes are not sent where they are written. Each one pushes
    /// `[endpoint, args]` into `$tx`, and one `await $atomic($tx)` at the
    /// end of the handler sends the whole list, which the server applies in
    /// a single store transaction. **Every durable write one handler
    /// performs commits together, in source order, or none of them does.**
    ///
    /// **No new syntax.** The handler was already a syntactic unit — `on
    /// click` and its indented block — so the transaction boundary is a
    /// production that already exists and the reserved-word budget of
    /// §14G.7.7 is untouched.
    ///
    /// **Why the batch can be built at all**, which is the part a general
    /// database client cannot do: §17.2.7's Command rule evaluates every
    /// right-hand side and every index in *this* region and ships them as
    /// arguments, so no value in the write set depends on reading the
    /// store. The whole transaction is therefore decided before the first
    /// write lands, and a non-interactive atomic batch — which is all Deno
    /// KV and DynamoDB offer — is sufficient. §17.7 records the
    /// expressiveness that rule cost; this is some of what it bought.
    ///
    /// **What it does not cover.** Client-signal writes in the same
    /// handler are not part of the transaction and cannot be: they are
    /// browser-local and there is nothing to roll them back with. And two
    /// handlers are two transactions — the unit is one handler, not one
    /// interaction.
    fn handler_source(&mut self, handler: &HirHandler) -> String {
        let parameter = handler
            .payload
            .map(|local| self.emitter.names.local(local).to_string())
            .unwrap_or_default();
        let single = self.emitter.hir.blocks[handler.body].stmts.len() == 1;
        let mut body = String::new();
        let mut statements = Statements {
            emitter: self.emitter,
            temporaries: 0,
            awaited: false,
            commands: 0,
            writes: Vec::new(),
            loops: 0,
            unbounded: false,
            // A handler is not a function declaration, so it is in no
            // cycle and has nothing to bounce to.
            bounce: None,
            // A handler is not a function body, so there is nothing for a
            // tail call to jump back to.
            tail: None,
        };
        statements.block(handler.body, 4, &mut body);
        let awaited = statements.awaited;
        let commands = statements.commands;
        let writes = std::mem::take(&mut statements.writes);
        let unbounded = statements.unbounded;

        if commands > 0 {
            // The write set, recorded for the manifest. A deploy adapter
            // reads it to check its target's batch cap at build time
            // instead of discovering it as a `TransactionCanceledException`
            // in production.
            self.emitter.transactions.push(crate::HandlerWrites {
                event: handler.event.clone(),
                writes,
                bounded: !unbounded,
            });
        }

        if commands > 0 {
            // `$tx` and `$atomic` are `$`-prefixed and therefore hygienic
            // against every name a program can spell: `$` is in neither
            // XID_Start nor XID_Continue.
            self.emitter.used.rpc.insert("atomic as $atomic");
            body = format!("    const $tx = [];\n{body}    await $atomic($tx);\n");
        }

        if !awaited {
            if single {
                let compact = body.trim().trim_end_matches(';');
                if !compact.contains('\n') && !compact.starts_with("return") {
                    return format!("({parameter}) => {compact}");
                }
            }
            return format!("({parameter}) => {{\n{body}  }}");
        }

        // The rejection needs a sink inside the handler: `on()` calls the
        // listener and drops what it returns, so an `async` arrow that
        // rejects would be an unhandled rejection — the same silence this
        // change exists to remove, one level up.
        self.emitter.used.rpc.insert("reportFailure as $failed");
        let indented: String = body
            .lines()
            .map(|line| {
                if line.is_empty() {
                    String::from("\n")
                } else {
                    format!("  {line}\n")
                }
            })
            .collect();
        format!(
            "async ({parameter}) => {{\n    try {{\n{indented}    }} catch ($e) {{\n      \
             $failed($e);\n    }}\n  }}"
        )
    }

    fn bind(&mut self, target: Address, kind: BindKind) {
        self.binds.push(Bind { target, kind });
    }
}

/// Every signal a `PasswordInput` anywhere under `nodes` binds.
///
/// Over the whole tree rather than one level, and over element children as
/// well as over the constructs that place nodes, because the leak and the
/// field can be written in either order and at any depth.
fn masked_signals(hir: &zdc_hir::Hir, nodes: &[HirNode], out: &mut BTreeSet<DefId>) {
    for node in nodes {
        match node {
            HirNode::Element(element) => masked_in_element(hir, element, out),
            HirNode::Each(each) => masked_signals(hir, &each.body, out),
            HirNode::When(when) => {
                for arm in &when.arms {
                    match &arm.body {
                        HirNodeArmBody::Show(element) => masked_in_element(hir, element, out),
                        HirNodeArmBody::Nodes(nodes) => masked_signals(hir, nodes, out),
                    }
                }
            }
            HirNode::If(conditional) => {
                masked_signals(hir, &conditional.then, out);
                if let Some(otherwise) = &conditional.otherwise {
                    masked_signals(hir, otherwise, out);
                }
            }
            HirNode::Scope(scope) => masked_signals(hir, &scope.body, out),
            HirNode::Handler(_) | HirNode::Children(_) => {}
        }
    }
}

fn masked_in_element(hir: &zdc_hir::Hir, element: &HirElement, out: &mut BTreeSet<DefId>) {
    if element.name == "PasswordInput" {
        // A two-way slot is a bare `state` name or it is a diagnostic, so
        // there is nothing deeper to walk.
        if let Some(expr) = leading_positional(element) {
            if let HirExprKind::Ref(Res::Def(def)) = hir.exprs[expr].kind {
                out.insert(def);
            }
        }
    }
    masked_signals(hir, &element.children, out);
}

/// An element's leading positional argument, if it wrote one.
fn leading_positional(element: &HirElement) -> Option<ExprId> {
    element.args.iter().find_map(|arg| match arg {
        HirArg::Positional(expr) => Some(*expr),
        HirArg::Named { .. } => None,
    })
}

/// Every element a run of nodes puts *directly* into its parent.
///
/// `each`, `if`, `when` and a scope are transparent: whatever they render
/// becomes a child of the element the construct was written under, with no
/// element of their own in between. A handler becomes a listener and never
/// reaches the DOM at all, and `children` was replaced by instantiation.
fn placed_elements<'n>(nodes: &'n [HirNode], out: &mut Vec<&'n HirElement>) {
    for node in nodes {
        match node {
            HirNode::Element(element) => out.push(element),
            HirNode::Each(each) => placed_elements(&each.body, out),
            HirNode::When(when) => {
                for arm in &when.arms {
                    match &arm.body {
                        HirNodeArmBody::Show(element) => out.push(element),
                        HirNodeArmBody::Nodes(nodes) => placed_elements(nodes, out),
                    }
                }
            }
            HirNode::If(conditional) => {
                placed_elements(&conditional.then, out);
                if let Some(otherwise) = &conditional.otherwise {
                    placed_elements(otherwise, out);
                }
            }
            HirNode::Scope(scope) => placed_elements(&scope.body, out),
            HirNode::Handler(_) | HirNode::Children(_) => {}
        }
    }
}

/// Push a hole's anchor pair into `out` and return the address of its
/// start comment, which is the node a walk names.
fn hole(path: &Address, index: usize, out: &mut Vec<Tpl>) -> Address {
    let mut target = path.clone();
    target.push(index);
    out.push(Tpl::Comment);
    out.push(Tpl::Comment);
    target
}

/// Whether this grammar's written form differs from what CSS is given.
///
/// Spelled out arm by arm rather than as a `matches!` with a wildcard, so
/// that a grammar added later has to answer the question rather than
/// inheriting an answer.
fn translated_when_folded(grammar: style::Grammar) -> bool {
    match grammar {
        style::Grammar::Keyword(_) | style::Grammar::Percent => true,
        // `Angle` belongs with `Lengths`, not with `Percent`. Sticking
        // `deg` on the end is concatenation, which `style_expression` does
        // at run time exactly as it does `px`; `Percent` is here because
        // `50` becomes `0.5`, and arithmetic is what a concatenation
        // cannot do. Answering otherwise would refuse a bound `rotate`,
        // which is the only reason the argument is worth having.
        style::Grammar::Angle
        | style::Grammar::Lengths
        | style::Grammar::Number
        | style::Grammar::Whole
        | style::Grammar::Colour
        | style::Grammar::Url
        | style::Grammar::Free => false,
    }
}

/// Everything an element accepts, for the diagnostic that refuses the rest.
///
/// The style arguments are listed with the rest rather than summarised.
/// The list is long and getting longer, and a reader who misspelled one
/// needs to see the spelling that exists; "and the style arguments" would
/// send them to a document instead.
fn permitted_arguments(shape: &elements::Shape) -> String {
    let mut names: Vec<&str> = elements::GLOBAL_ARGUMENTS.to_vec();
    names.extend(shape.arguments.iter().copied());
    names.extend(elements::STYLE_ARGUMENTS.iter().map(|(name, _)| *name));
    names.sort_unstable();
    names.dedup();
    let prefixes: Vec<&str> = elements::PREFIXES
        .iter()
        .map(|(prefix, _)| *prefix)
        .collect();
    format!(
        "{}. A style argument may also carry one of {}, as in `hoverBackground` or \
         `narrowDisplay`, which applies it in that circumstance alone",
        english_list(&names),
        english_list(&prefixes)
    )
}

/// The `'static` spelling of a built-in's name, so a parent can be tracked
/// without an allocation per element.
fn shape_name(name: &str) -> Option<&'static str> {
    elements::BUILT_INS
        .iter()
        .find(|built_in| **built_in == name)
        .copied()
}

/// `` `a` ``, `` `b` `` and `` `c` `` — the phrasing every list in a
/// diagnostic uses.
fn english_list(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("`{item}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

fn named_argument_of(element: &HirElement, wanted: &str) -> Option<ExprId> {
    element.args.iter().find_map(|arg| match arg {
        HirArg::Named { name, value } if name == wanted => Some(*value),
        _ => None,
    })
}

/// Replace rather than repeat: `elements.js` spreads a program's arguments
/// over the built-in ones, so `Input type is "email"` wins. Two attributes
/// of the same name in markup would silently keep the first instead.
fn set_attribute(attributes: &mut Vec<(String, String)>, name: &str, value: String) {
    match attributes.iter_mut().find(|(existing, _)| existing == name) {
        Some(slot) => slot.1 = value,
        None => attributes.push((name.to_string(), value)),
    }
}

/// An argument name as the field `scene.js` reads it.
///
/// The same rename `elements::named_argument` performs for the DOM, and
/// deliberately a second table rather than a call into that one: the DOM
/// spelling of a stroke width is `stroke-width`, which is not a
/// JavaScript property name, and a shared table would have had to carry
/// both anyway. Four lines of duplication against a hyphen that only
/// means something to CSS.
fn draw_field(name: &str) -> Option<&'static str> {
    Some(match name {
        "outline" => "d",
        "x" => "cx",
        "y" => "cy",
        "radius" => "r",
        "fromX" => "x1",
        "fromY" => "y1",
        "toX" => "x2",
        "toY" => "y2",
        "fill" => "fill",
        "stroke" => "stroke",
        "strokeWidth" => "strokeWidth",
        "opacity" => "opacity",
        _ => return None,
    })
}

fn getter_source(operand: Operand) -> String {
    match operand {
        Operand::Literal(literal) => literal.as_js(),
        Operand::Static(value) => value,
        Operand::Reactive(getter) => getter,
    }
}

fn print_markup(node: &Tpl, out: &mut String) {
    match node {
        Tpl::Text(text) => out.push_str(&js::html_text(text)),
        Tpl::Comment => out.push_str("<!---->"),
        Tpl::Element {
            tag,
            attributes,
            children,
        } => {
            out.push('<');
            out.push_str(tag);
            for (name, value) in attributes {
                out.push(' ');
                out.push_str(name);
                if !value.is_empty() {
                    out.push('=');
                    out.push_str(&js::html_attribute(value).to_string());
                }
            }
            out.push('>');
            // A void element has no end tag and no children; the shape
            // table never gives one any.
            if is_void(tag) {
                return;
            }
            for child in children {
                print_markup(child, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
    }
}

fn is_void(tag: &str) -> bool {
    matches!(tag, "input" | "br" | "hr" | "img" | "meta" | "link")
}

// --- P3: path scheduling --------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    FirstChild,
    NextSibling,
}

impl Step {
    fn property(self) -> &'static str {
        match self {
            Step::FirstChild => "firstChild",
            Step::NextSibling => "nextSibling",
        }
    }
}

/// A region flattened into the graph the walk actually traverses:
/// `firstChild` and `nextSibling` edges, and nothing else. There is no edge
/// upward, which is why a base is chosen by shortest path rather than by
/// tree ancestry.
struct Graph {
    /// Pre-order, so index order is document order.
    addresses: Vec<Address>,
    first_child: Vec<Option<usize>>,
    next_sibling: Vec<Option<usize>>,
}

impl Graph {
    fn build(roots: &[Tpl]) -> Graph {
        let mut graph = Graph {
            addresses: Vec::new(),
            first_child: Vec::new(),
            next_sibling: Vec::new(),
        };
        let mut path = Vec::new();
        graph.add_siblings(roots, &mut path);
        graph
    }

    fn add_siblings(&mut self, nodes: &[Tpl], path: &mut Address) -> Vec<usize> {
        let mut ids = Vec::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            path.push(index);
            let id = self.addresses.len();
            self.addresses.push(path.clone());
            self.first_child.push(None);
            self.next_sibling.push(None);
            ids.push(id);

            if let Tpl::Element { children, .. } = node {
                let child_ids = self.add_siblings(children, path);
                self.first_child[id] = child_ids.first().copied();
            }
            path.pop();
        }
        for window in ids.windows(2) {
            self.next_sibling[window[0]] = Some(window[1]);
        }
        ids
    }

    fn id_of(&self, address: &[usize]) -> Option<usize> {
        self.addresses.iter().position(|a| a == address)
    }

    /// The shortest walk from `from` to `to`, or `None` if `to` is not
    /// reachable — a walk can descend and go right, never left or up.
    fn route(&self, from: usize, to: usize) -> Option<Vec<Step>> {
        if from == to {
            return Some(Vec::new());
        }
        let mut previous: Vec<Option<(usize, Step)>> = vec![None; self.addresses.len()];
        let mut seen = vec![false; self.addresses.len()];
        let mut queue = VecDeque::from([from]);
        seen[from] = true;

        while let Some(node) = queue.pop_front() {
            for (next, step) in [
                (self.first_child[node], Step::FirstChild),
                (self.next_sibling[node], Step::NextSibling),
            ] {
                let Some(next) = next else { continue };
                if seen[next] {
                    continue;
                }
                seen[next] = true;
                previous[next] = Some((node, step));
                if next == to {
                    let mut steps = Vec::new();
                    let mut cursor = to;
                    while let Some((parent, step)) = previous[cursor] {
                        steps.push(step);
                        cursor = parent;
                    }
                    steps.reverse();
                    return Some(steps);
                }
                queue.push_back(next);
            }
        }
        None
    }
}

struct Site {
    anchor: usize,
    suffix: Vec<Step>,
    kind: BindKind,
}

/// P3 and P5: the walk that names nodes and the statements that bind them.
///
/// One `Emission` covers a whole module, because a region nested in a hole
/// needs its own template constant and its own walk locals, and both
/// numbering schemes have to stay unique across the file.
pub struct Emission<'u> {
    used: &'u mut RuntimeImports,
    /// One entry per `$tN`, in the order the constants are emitted. The
    /// root region is always index 0.
    templates: Vec<(String, bool)>,
    fragments: usize,
    /// `$nN` is numbered across the whole module rather than per region,
    /// so a nested region's walk locals never shadow the ones the walk it
    /// sits inside is still holding.
    locals: usize,
    /// Whether any `each` was emitted, so `$byPosition` is declared exactly
    /// when something calls it (spec §16.6).
    by_position: bool,
}

impl<'u> Emission<'u> {
    pub fn new(used: &'u mut RuntimeImports) -> Emission<'u> {
        Emission {
            used,
            templates: Vec::new(),
            fragments: 0,
            locals: 0,
            by_position: false,
        }
    }

    /// The template constants the module declares, in `$tN` order.
    pub fn templates(&self) -> &[(String, bool)] {
        &self.templates
    }

    /// Whether the module needs the positional key function.
    pub fn needs_by_position(&self) -> bool {
        self.by_position
    }

    /// The document's own root: the whole of `main`'s body, including the
    /// line that hands the tree back.
    ///
    /// **The return is emitted here because it depends on how the root was
    /// obtained.** A root bound to the container is already mounted and
    /// has nothing to do; a root that cloned a fragment — which is every
    /// region with no markup of its own, an `each` at the top of a view
    /// among them — must mount *after* its bindings, because `mount`
    /// inserts a fragment's children and empties the fragment.
    pub fn root_instance(&mut self, region: &Region, fragment: &str, indent: usize) -> String {
        let adopting = self.can_adopt(region);
        let mut out = self.instance_with(region, fragment, indent, adopting);
        let pad = " ".repeat(indent);
        if adopting {
            out.push_str(&format!("{pad}return {fragment};\n"));
        } else {
            self.used.dom.insert("mount");
            out.push_str(&format!("{pad}return mount({fragment}, container);\n"));
        }
        out
    }

    /// Whether this root has markup a prerender could have painted.
    ///
    /// A region that is nothing but anchors has none: `anchors()` builds
    /// two comments and there is no template to compare a container
    /// against, so it clones and mounts exactly as it always did.
    fn can_adopt(&self, region: &Region) -> bool {
        !region.roots.is_empty() && !region.is_only_anchors()
    }

    fn instance_with(
        &mut self,
        region: &Region,
        fragment: &str,
        indent: usize,
        adopting: bool,
    ) -> String {
        let mut out = if adopting {
            self.adopt_template(region, fragment, indent)
        } else {
            self.clone_template(region, fragment, indent)
        };
        out.push_str(&self.locals(region, indent));
        out.push_str(&self.region(region, fragment, indent));
        out
    }

    /// The signals this instance owns, declared before anything reads them.
    fn locals(&mut self, region: &Region, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let mut out = String::new();
        for local in &region.locals {
            if let Some(clock) = local.clock {
                let call = crate::clock_call(self.used, clock);
                out.push_str(&format!("{pad}const {} = {call};\n", local.getter));
                continue;
            }
            if local.is_source {
                self.used.signal.insert("signal");
                match &local.setter {
                    Some(setter) => out.push_str(&format!(
                        "{pad}const [{}, {setter}] = signal({});\n",
                        local.getter, local.value
                    )),
                    None => out.push_str(&format!(
                        "{pad}const [{}] = signal({});\n",
                        local.getter, local.value
                    )),
                }
            } else {
                self.used.signal.insert("derived");
                out.push_str(&format!(
                    "{pad}const {} = derived(() => {});\n",
                    local.getter,
                    js::arrow_body(&local.value)
                ));
            }
        }
        out
    }

    /// The statement that produces a fresh copy of a region's markup.
    /// The same as [`Emission::clone_template`], except that the root it
    /// binds against is whatever is already in the container.
    ///
    /// A build paints the first frame into the document, so on most loads
    /// the nodes exist before this module runs. `adopt` hands those back
    /// in the clone's place and every address below walks to the same
    /// node it would have — the markup came from this same template.
    ///
    /// A region with nothing to clone is untouched: there is no markup to
    /// adopt and `anchors()` builds its two comments as it always did.
    fn adopt_template(&mut self, region: &Region, fragment: &str, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let svg = region.is_svg();
        let index = self.templates.len();
        self.templates.push((region.html(), svg));
        if svg {
            self.used.vector.insert("templateSvg");
        } else {
            self.used.dom.insert("template");
        }
        // **The container itself, when it already holds the paint.** Every
        // address below is a `firstChild`/`nextSibling` walk, and the
        // container's children are the template's roots — the same nodes
        // a clone would have had, because the markup came from this
        // template. So there is nothing to move: binding against the
        // container binds against what is already on screen.
        //
        // The empty case clones *and mounts here*, which is why the root
        // needs no `mount` call after its bindings. Mounting early costs
        // the painted path nothing — its nodes are in the document
        // already — and costs the empty path nothing either, because the
        // bindings that follow run in the same task as the load and so
        // before any paint. `dom.js` keeps `mount` exactly as it was.
        self.used.dom.insert("mount");
        // **The container is the root in both branches**, and it has to
        // be: `mount` inserts a fragment's children and empties the
        // fragment, so a root bound to the clone would walk into
        // something with no children left. Binding to the container is
        // the same walk either way — its children are the template's
        // roots, painted or cloned.
        //
        // Mounting before the bindings costs nothing. The painted path's
        // nodes are in the document already, and the cloned path's
        // bindings still run in the task that loaded the module, so
        // before any paint — which is the same argument the template's
        // deliberate space has always rested on.
        format!(
            "{pad}if (!container.firstChild) mount($t{index}(), container);\n\
             {pad}const {fragment} = container;\n"
        )
    }

    fn clone_template(&mut self, region: &Region, fragment: &str, indent: usize) -> String {
        let pad = " ".repeat(indent);
        if region.roots.is_empty() {
            return format!("{pad}const {fragment} = document.createDocumentFragment();\n");
        }
        // A region that is nothing but a hole has no markup worth parsing.
        if region.is_only_anchors() {
            self.used.dom.insert("anchors");
            return format!("{pad}const {fragment} = anchors();\n");
        }
        let index = self.templates.len();
        let svg = region.is_svg();
        self.templates.push((region.html(), svg));
        if svg {
            self.used.vector.insert("templateSvg");
        } else {
            self.used.dom.insert("template");
        }
        format!("{pad}const {fragment} = $t{index}();\n")
    }

    /// A region as the body of an arrow function, for an `each` row or a
    /// `when` arm. The parameters are written out exactly, never with a
    /// default or a rest, so `Function.prototype.length` is the arity.
    fn closure(&mut self, region: &Region, params: &[String], indent: usize) -> String {
        let fragment = format!("$r{}", self.fragments);
        self.fragments += 1;
        let inner = indent + 2;
        let pad = " ".repeat(indent);
        let inner_pad = " ".repeat(inner);

        let mut out = format!("({}) => {{\n", params.join(", "));
        out.push_str(&self.clone_template(region, &fragment, inner));
        out.push_str(&self.locals(region, inner));
        out.push_str(&self.region(region, &fragment, inner));
        out.push_str(&format!("{inner_pad}return {fragment};\n{pad}}}"));
        out
    }

    fn region(&mut self, region: &Region, fragment: &str, indent: usize) -> String {
        let graph = Graph::build(&region.roots);
        let pad = " ".repeat(indent);
        let mut out = String::new();

        // Each bind's *anchor* is the node the walk names. A text-node
        // target is addressed off its parent, which is what makes the
        // emission `bindText($n1.firstChild, count)` rather than naming the
        // text node; an element and a hole anchor name themselves.
        let mut sites: Vec<Site> = Vec::new();
        for bind in &region.binds {
            let Some(target) = graph.id_of(&bind.target) else {
                continue;
            };
            if matches!(
                node_at(&region.roots, &bind.target),
                Some(Tpl::Element { .. }) | Some(Tpl::Comment)
            ) {
                sites.push(Site {
                    anchor: target,
                    suffix: Vec::new(),
                    kind: bind.kind.clone(),
                });
                continue;
            }
            let Some(parent) = graph.id_of(&bind.target[..bind.target.len() - 1]) else {
                continue;
            };
            let suffix = graph
                .route(parent, target)
                .expect("a child is always reachable from its parent");
            sites.push(Site {
                anchor: parent,
                suffix,
                kind: bind.kind.clone(),
            });
        }

        // Name every anchor, plus every root a walk has to pass through to
        // reach one. A region with no bindings names nothing at all.
        let mut named: Vec<usize> = sites.iter().map(|site| site.anchor).collect();
        if !named.is_empty() {
            let roots: Vec<usize> = (0..graph.addresses.len())
                .filter(|id| graph.addresses[*id].len() == 1)
                .collect();
            for root in roots {
                if sites.iter().any(|site| is_under(&graph, root, site.anchor)) {
                    named.push(root);
                }
            }
        }
        named.sort_unstable();
        named.dedup();

        let mut assigned: Vec<(usize, String)> = Vec::new();
        for id in named {
            let name = format!("$n{}", self.locals);
            self.locals += 1;
            let chain = shortest_chain(&graph, &assigned, fragment, id);
            out.push_str(&format!("{pad}const {name} = {chain};\n"));
            assigned.push((id, name));
        }

        // Bindings in document order, and in declaration order within a node.
        let mut order: Vec<usize> = (0..sites.len()).collect();
        order.sort_by_key(|index| sites[*index].anchor);

        for index in order {
            let kind = sites[index].kind.clone();
            let mut target = assigned
                .iter()
                .find(|(node, _)| *node == sites[index].anchor)
                .map(|(_, name)| name.clone())
                .expect("every anchor was named");
            for step in &sites[index].suffix {
                target.push('.');
                target.push_str(step.property());
            }
            out.push_str(&self.attach(&kind, &target, indent));
        }

        // After the bindings, so the view a key handler writes into is
        // already wired when the first keystroke can arrive.
        for (key, handler) in &region.keys {
            self.used.keys.insert("onKey");
            out.push_str(&format!("{pad}onKey({}, {handler});\n", js::string(key)));
        }

        out
    }

    fn attach(&mut self, kind: &BindKind, target: &str, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match kind {
            BindKind::Text(getter) => {
                self.used.dom.insert("bindText");
                format!("{pad}bindText({target}, {getter});\n")
            }
            BindKind::TextOnce(value) => {
                format!("{pad}{target}.nodeValue = String({value});\n")
            }
            BindKind::MarkupOnce(value) => {
                self.used.rendered.insert("markup");
                format!("{pad}markup({target}, {value});\n")
            }
            BindKind::Markup(getter) => {
                self.used.rendered.insert("bindMarkup");
                format!("{pad}bindMarkup({target}, {getter});\n")
            }
            // Every name below is a string *argument*, so it is
            // written with `js::string` rather than between two
            // apostrophes. An attribute name and an event name are both
            // program text — `Text foo is "x"` names the attribute — and
            // they are safe today only because the lexer's identifier rule
            // happens to exclude an apostrophe. That is an accident of the
            // current grammar, not a property of this emitter.
            BindKind::Attribute { name, getter } => {
                self.used.dom.insert("bindAttr");
                let name = js::string(name);
                format!("{pad}bindAttr({target}, {name}, {getter});\n")
            }
            BindKind::AttributeOnce { name, value } => {
                let name = js::string(name);
                format!("{pad}{target}.setAttribute({name}, String({value}));\n")
            }
            // The helper itself is asked for where the slot is lowered,
            // beside `$optionalNumber`; what belongs here is the runtime
            // symbol the helper calls, exactly as its neighbours declare
            // theirs.
            BindKind::Scene {
                renderer,
                view_box,
                draws,
            } => {
                self.used.scene.insert("scene");
                format!(
                    "{pad}scene({target}, {{ renderer: {renderer}, viewBox: {view_box} }}, () => [{draws}]);\n"
                )
            }
            BindKind::NumberField(getter) => {
                self.used.signal.insert("effect");
                format!("{pad}$numberField({target}, {getter});\n")
            }
            // Two runtime symbols rather than one: the helper opens and
            // closes inside an `effect` and listens for `close` through
            // `on`, so a bundle that imported only the first would throw
            // on the listener it never linked.
            BindKind::Modal { getter, setter } => {
                self.used.signal.insert("effect");
                self.used.dom.insert("on");
                format!("{pad}$modal({target}, {getter}, {setter});\n")
            }
            BindKind::StyleOnce { property, value } => {
                // Through `js::string`, as its three neighbours are. The
                // property is a `&'static str` off the emitter's own table
                // today, so this changes no byte — but the rule that owns
                // the quotes is the reason it stays that way, and the one
                // site here that wrote its own was the shape three
                // injection holes have already had.
                let property = js::string(property);
                format!("{pad}{target}.style.setProperty({property}, String({value}));\n")
            }
            BindKind::Style { property, getter } => {
                self.used.dom.insert("bindStyle");
                let property = js::string(property);
                format!("{pad}bindStyle({target}, {property}, {getter});\n")
            }
            BindKind::Listener { event, handler } => {
                self.used.dom.insert("on");
                let event = js::string(event);
                format!("{pad}on({target}, {event}, {handler});\n")
            }
            // The pair of comments is `target` and its next sibling, so the
            // region's extent is known without wrapping it in an element
            // the program never asked for.
            BindKind::Each { list, binder, body } => {
                self.used.reconcile.insert("eachInto");
                self.by_position = true;
                let render = self.closure(body, std::slice::from_ref(binder), indent);
                format!(
                    "{pad}eachInto({target}, {target}.nextSibling, {list}, $byPosition, \
                     {render});\n"
                )
            }
            BindKind::When { scrutinee, arms } => {
                self.used.dom.insert("whenInto");
                let mut written = String::new();
                for arm in arms {
                    let closure = self.closure(&arm.body, &arm.binders, indent + 2);
                    written.push_str(&format!("{pad}  {}: {closure},\n", js::string(&arm.name)));
                }
                format!(
                    "{pad}whenInto({target}, {target}.nextSibling, {scrutinee}, {{\n{written}\
                     {pad}}});\n"
                )
            }
            BindKind::If {
                condition,
                then,
                otherwise,
            } => {
                self.used.dom.insert("ifInto");
                let then = self.closure(then, &[], indent);
                let otherwise = match otherwise {
                    Some(region) => self.closure(region, &[], indent),
                    None => "null".to_string(),
                };
                format!(
                    "{pad}ifInto({target}, {target}.nextSibling, {condition}, {then}, \
                     {otherwise});\n"
                )
            }
            // `{target}` and not `{target}.nextSibling`: the node itself is
            // the boundary, so there is no region to delimit.
            //
            // The props object is built inside a thunk, so every read in it
            // is a read the runtime's effect performs — which is what makes
            // a signal write reach `update` and reach nothing else. Keys are
            // quoted through the escaper: a parameter name is a ZDeceptron
            // identifier and this emitter does not decide whether that is
            // also a JavaScript one.
            BindKind::Foreign {
                callee,
                props,
                declared,
            } => {
                self.used.lifecycle.insert("foreign");
                let written = props
                    .iter()
                    .map(|(name, value)| format!("{}: {value}", js::string(name)))
                    .collect::<Vec<_>>()
                    .join(", ");
                // The declaration's name goes through the same escaper as
                // everything else the program wrote: it is an identifier
                // here and this emitter does not decide whether it is also
                // a JavaScript one.
                let named = js::string(declared);
                format!("{pad}foreign({target}, {callee}, () => ({{{written}}}), {named});\n")
            }
        }
    }
}

/// The chain to `id` from whichever base is nearest.
///
/// Always preferring the nearest already-named node over re-walking from
/// the root is what makes `counter.zd`'s walk five statements rather than
/// five independent chains: fewer property loads, and every receiver comes
/// from a clone of a fixed template, so each access site sees exactly one
/// hidden class.
fn shortest_chain(
    graph: &Graph,
    assigned: &[(usize, String)],
    fragment: &str,
    id: usize,
) -> String {
    let mut best: Option<(usize, String, Vec<Step>)> = None;
    for (base, name) in assigned {
        let Some(steps) = graph.route(*base, id) else {
            continue;
        };
        // A tie goes to the most recently named base, which is the one
        // document order has just walked past.
        if best
            .as_ref()
            .is_none_or(|(length, _, _)| steps.len() <= *length)
        {
            best = Some((steps.len(), name.clone(), steps));
        }
    }

    // The fragment reaches root 0 by `firstChild`, and everything else
    // from there, so it is always a fallback and sometimes the shortest.
    if let Some(steps) = graph.route(0, id) {
        let length = steps.len() + 1;
        if best.as_ref().is_none_or(|(best, _, _)| length < *best) {
            return chain_from(format!("{fragment}.firstChild"), &steps);
        }
    }

    let (_, name, steps) = best.expect("a node is reachable from the fragment");
    chain_from(name, &steps)
}

fn chain_from(base: String, steps: &[Step]) -> String {
    let mut chain = base;
    for step in steps {
        chain.push('.');
        chain.push_str(step.property());
    }
    chain
}

/// Whether `anchor` sits strictly inside the subtree rooted at `root`.
fn is_under(graph: &Graph, root: usize, anchor: usize) -> bool {
    let root_address = &graph.addresses[root];
    let anchor_address = &graph.addresses[anchor];
    anchor_address.len() > root_address.len()
        && anchor_address[..root_address.len()] == root_address[..]
}

fn node_at<'t>(roots: &'t [Tpl], address: &[usize]) -> Option<&'t Tpl> {
    let (first, rest) = address.split_first()?;
    let mut node = roots.get(*first)?;
    for index in rest {
        let Tpl::Element { children, .. } = node else {
            return None;
        };
        node = children.get(*index)?;
    }
    Some(node)
}
