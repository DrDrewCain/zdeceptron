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

use std::collections::{BTreeSet, VecDeque};

use zdc_hir::{
    DefKind, ExprId, HirArg, HirElement, HirExprKind, HirHandler, HirNode, HirNodeArmBody, Res,
};

use crate::elements::{self, Named, Slot};
use crate::expr::{Emitter, Literal, Operand};
use crate::js;
use crate::stmt::Statements;
use crate::styles::Styles;

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
    Attribute {
        name: String,
        getter: String,
    },
    AttributeOnce {
        name: String,
        value: String,
    },
    Style {
        property: String,
        getter: String,
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

/// One template's worth of markup and the bindings attached to it.
#[derive(Debug, Clone)]
pub struct Region {
    roots: Vec<Tpl>,
    binds: Vec<Bind>,
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

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
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
#[derive(Default)]
pub struct RuntimeImports {
    pub signal: BTreeSet<&'static str>,
    pub dom: BTreeSet<&'static str>,
    /// The `$`-prefixed prelude helpers this module used (§17.4.7).
    ///
    /// Not an import: §16.3.12 assertion A requires a bundle to import no
    /// ZDeceptron-generated module, so these are declared inline in the
    /// preamble — which is also what lets a program that never indexes a
    /// map ship without `$mapAt`.
    pub helpers: BTreeSet<&'static str>,
}

// --- P1 and P2: lowering and partition ------------------------------------

pub struct Lowering<'a, 'h> {
    emitter: &'a mut Emitter<'h>,
    styles: &'a mut Styles,
    binds: Vec<Bind>,
}

impl<'a, 'h> Lowering<'a, 'h> {
    pub fn new(emitter: &'a mut Emitter<'h>, styles: &'a mut Styles) -> Lowering<'a, 'h> {
        Lowering {
            emitter,
            styles,
            binds: Vec::new(),
        }
    }

    pub fn region(mut self, nodes: &[HirNode]) -> Region {
        let mut path = Vec::new();
        let roots = self.nodes(nodes, &mut path, 0);
        Region {
            roots,
            binds: self.binds,
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
                HirNode::Handler(handler) => {
                    self.emitter.error(
                        "`on` must be written inside the element it handles, indented under it.",
                        handler.span,
                    );
                }
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
            }
        }
        out
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
        }
        .region(nodes)
    }

    fn element(&mut self, element: &HirElement, path: &mut Address) -> Tpl {
        let Some(shape) = elements::shape(&element.name) else {
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

        // `Checkbox label is ...` wraps the box in a labelled row, so every
        // binding on the box sits one level below the element's own address.
        let labelled = shape.slot == Slot::Checked && named_argument_of(element, "label").is_some();
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
        let mut declarations: Vec<(String, String)> = Vec::new();
        let mut children: Vec<Tpl> = Vec::new();

        self.leading_argument(element, shape.slot, &inner, &mut children);

        if let Some(literal) = shape.literal_text {
            children.push(Tpl::Text(literal.to_string()));
        }

        for arg in &element.args {
            let HirArg::Named { name, value } = arg else {
                continue;
            };
            if name == "label" {
                if !labelled {
                    self.emitter.error(
                        format!("`{}` does not use `label`.", element.name),
                        element.span,
                    );
                }
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
            );
        }

        // A static style set folds into one generated class and costs
        // nothing at runtime (spec §6, §16.3.11).
        if !declarations.is_empty() {
            classes.push(self.styles.intern(declarations));
        }
        if !classes.is_empty() {
            set_attribute(&mut attributes, "class", classes.join(" "));
        }

        // Handlers are children in the HIR and listeners in the emission,
        // so they never reach the markup.
        for child in &element.children {
            if let HirNode::Handler(handler) = child {
                self.listener(element, shape.slot, handler, &inner);
            }
        }

        let element_children: Vec<HirNode> = element
            .children
            .iter()
            .filter(|child| !matches!(child, HirNode::Handler(_)))
            .cloned()
            .collect();
        if !element_children.is_empty() {
            if shape.children {
                let start = children.len();
                let mut child_path = inner.clone();
                let lowered = self.nodes(&element_children, &mut child_path, start);
                children.extend(lowered);
            } else {
                self.emitter.error(
                    format!("`{}` shows one value and takes no children.", element.name),
                    element.span,
                );
            }
        }

        let node = Tpl::Element {
            tag: shape.tag,
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

    fn leading_argument(
        &mut self,
        element: &HirElement,
        slot: Slot,
        target: &Address,
        children: &mut Vec<Tpl>,
    ) {
        let mut positionals = element.args.iter().filter_map(|arg| match arg {
            HirArg::Positional(expr) => Some(*expr),
            HirArg::Named { .. } => None,
        });
        let leading = positionals.next();
        if positionals.next().is_some() {
            self.emitter.error(
                format!("`{}` takes at most one leading argument.", element.name),
                element.span,
            );
        }

        match (slot, leading) {
            (Slot::None, Some(_)) => self.emitter.error(
                format!(
                    "`{}` has no leading argument in `elements.js`, yet four checked-in examples \
                     write one. §16.3.6 recommends giving `Row` and `Column` a leading text slot \
                     as `Button` already has; until that is ratified in §4.4 the compiler refuses \
                     rather than inventing the semantics.",
                    element.name
                ),
                element.span,
            ),
            (Slot::Text, Some(expr)) => {
                let operand = self.emitter.operand(expr);
                self.text_child(operand, children, target);
            }
            (Slot::Text, None) => self.emitter.error(
                format!("`{}` needs the text it shows.", element.name),
                element.span,
            ),
            (Slot::Value | Slot::Checked, Some(expr)) => {
                let attribute = if slot == Slot::Value {
                    "value"
                } else {
                    "checked"
                };
                self.two_way(element, expr, attribute, target);
            }
            (Slot::Value | Slot::Checked, None) => self.emitter.error(
                format!("`{}` needs the state it binds to.", element.name),
                element.span,
            ),
            (Slot::Message, Some(_)) => self.emitter.error(
                "`ErrorBar` takes its text as `message is ...`, not as a leading argument.",
                element.span,
            ),
            (Slot::None | Slot::Message, None) => {}
        }
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
        declarations: &mut Vec<(String, String)>,
    ) {
        match elements::named_argument(name) {
            Named::Consumed => self.emitter.error(
                format!("`{}` does not use `{name}`.", element.name),
                element.span,
            ),
            Named::Class => match operand {
                Operand::Literal(literal) => classes.push(literal.as_text()),
                other => {
                    let base = classes.join(" ");
                    let getter = getter_source(other);
                    self.bind(
                        target.clone(),
                        BindKind::Attribute {
                            name: "class".to_string(),
                            getter: format!("() => '{base} ' + ({getter})()"),
                        },
                    );
                }
            },
            Named::Style { property, px } => match operand {
                Operand::Literal(literal) => {
                    let value = if px {
                        format!("{}px", literal.as_text())
                    } else {
                        literal.as_text()
                    };
                    declarations.push((property.to_string(), value));
                }
                other => {
                    let getter = getter_source(other);
                    let getter = if px {
                        format!("() => ({getter})() + 'px'")
                    } else {
                        getter
                    };
                    self.bind(
                        target.clone(),
                        BindKind::Style {
                            property: property.to_string(),
                            getter,
                        },
                    );
                }
            },
            Named::Attribute(attribute) => match operand {
                // `setAttribute` removes on `false` and sets the empty
                // string on `true`, so a static one is markup or nothing.
                Operand::Literal(Literal::Truth(false)) => {}
                Operand::Literal(Literal::Truth(true)) => {
                    set_attribute(attributes, &attribute, String::new());
                }
                Operand::Literal(literal) => {
                    set_attribute(attributes, &attribute, literal.as_text());
                }
                Operand::Static(value) => self.bind(
                    target.clone(),
                    BindKind::AttributeOnce {
                        name: attribute,
                        value,
                    },
                ),
                Operand::Reactive(getter) => self.bind(
                    target.clone(),
                    BindKind::Attribute {
                        name: attribute,
                        getter,
                    },
                ),
            },
        }
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

    /// `Input name` and `Checkbox done`: one attribute binding plus one
    /// listener, both on the cloned node.
    fn two_way(&mut self, element: &HirElement, expr: ExprId, attribute: &str, target: &Address) {
        let span = self.emitter.hir.exprs[expr].span;
        let HirExprKind::Ref(Res::Def(def)) = self.emitter.hir.exprs[expr].kind else {
            self.emitter.error(
                format!(
                    "`{}` binds two-way, so it needs a `state` name rather than an expression \
                     (spec §14B.5).",
                    element.name
                ),
                span,
            );
            return;
        };
        let DefKind::Signal(signal) = &self.emitter.hir.defs[def].kind else {
            self.emitter.error(
                format!("`{}` binds two-way and needs `state`.", element.name),
                span,
            );
            return;
        };
        let placement = signal.placement;
        let is_source = signal.is_source;
        let declared = self.emitter.hir.defs[def].name.clone();

        if !is_source {
            self.emitter.error(
                format!(
                    "`{declared}` is declared with `from`, so the compiler recomputes it. A \
                     two-way binding needs a `starting` signal."
                ),
                span,
            );
            return;
        }
        if placement != zdc_ast::Placement::Client {
            self.emitter.error(
                format!(
                    "`{declared}` is {placement:?}-placed, and a keystroke must not silently \
                     become a network write (spec §14B.5)."
                ),
                span,
            );
            return;
        }

        let getter = self.emitter.names.def(def).to_string();
        let Some(setter) = self.emitter.names.setter(def).map(str::to_string) else {
            self.emitter.error(
                format!("`{declared}` is bound two-way but was given no setter."),
                element.span,
            );
            return;
        };

        let (event, handler) = if attribute == "value" {
            ("input", format!("(e) => {setter}(e.target.value)"))
        } else {
            ("change", format!("(e) => {setter}(e.target.checked)"))
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

    fn listener(
        &mut self,
        element: &HirElement,
        slot: Slot,
        handler: &HirHandler,
        target: &Address,
    ) {
        if (slot == Slot::Value && handler.event == "input")
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
        let source = self.handler_source(handler);
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
    fn handler_source(&mut self, handler: &HirHandler) -> String {
        let single = self.emitter.hir.blocks[handler.body].stmts.len() == 1;
        let mut body = String::new();
        let mut statements = Statements {
            emitter: self.emitter,
            temporaries: 0,
            // A handler is not a function body, so there is nothing for a
            // tail call to jump back to.
            tail: None,
        };
        statements.block(handler.body, 4, &mut body);

        if single {
            let compact = body.trim().trim_end_matches(';');
            if !compact.contains('\n') && !compact.starts_with("return") {
                return format!("() => {compact}");
            }
        }
        format!("() => {{\n{body}  }}")
    }

    fn bind(&mut self, target: Address, kind: BindKind) {
        self.binds.push(Bind { target, kind });
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
                    out.push_str("=\"");
                    out.push_str(&js::html_attribute(value));
                    out.push('"');
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
    templates: Vec<String>,
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
    pub fn templates(&self) -> &[String] {
        &self.templates
    }

    /// Whether the module needs the positional key function.
    pub fn needs_by_position(&self) -> bool {
        self.by_position
    }

    /// Build one instance of `region` into `fragment` and bind it.
    pub fn instance(&mut self, region: &Region, fragment: &str, indent: usize) -> String {
        let mut out = self.clone_template(region, fragment, indent);
        out.push_str(&self.region(region, fragment, indent));
        out
    }

    /// The statement that produces a fresh copy of a region's markup.
    fn clone_template(&mut self, region: &Region, fragment: &str, indent: usize) -> String {
        let pad = " ".repeat(indent);
        if region.is_empty() {
            return format!("{pad}const {fragment} = document.createDocumentFragment();\n");
        }
        // A region that is nothing but a hole has no markup worth parsing.
        if region.is_only_anchors() {
            self.used.dom.insert("anchors");
            return format!("{pad}const {fragment} = anchors();\n");
        }
        let index = self.templates.len();
        self.templates.push(region.html());
        self.used.dom.insert("template");
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
            BindKind::Attribute { name, getter } => {
                self.used.dom.insert("bindAttr");
                format!("{pad}bindAttr({target}, '{name}', {getter});\n")
            }
            BindKind::AttributeOnce { name, value } => {
                format!("{pad}{target}.setAttribute('{name}', String({value}));\n")
            }
            BindKind::Style { property, getter } => {
                self.used.dom.insert("bindStyle");
                format!("{pad}bindStyle({target}, '{property}', {getter});\n")
            }
            BindKind::Listener { event, handler } => {
                self.used.dom.insert("on");
                format!("{pad}on({target}, '{event}', {handler});\n")
            }
            // The pair of comments is `target` and its next sibling, so the
            // region's extent is known without wrapping it in an element
            // the program never asked for.
            BindKind::Each { list, binder, body } => {
                self.used.dom.insert("eachInto");
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
