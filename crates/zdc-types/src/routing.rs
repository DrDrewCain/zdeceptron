//! Routes, checked and enumerated (spec §14G.2).
//!
//! Three jobs, and they are one pass because the third needs the first
//! two:
//!
//! 1. **`static` is a build-time constant.** A `static` signal is
//!    evaluated on the build host and inlined (§14C.3b), so its
//!    initialiser has to be something the build host can evaluate with no
//!    runtime. Today that is literals and other `static` signals; when the
//!    FFI lands it becomes literals, other `static` signals, and `foreign`
//!    calls (§17.2.10 E0323).
//! 2. **A route parameter's `in` collection is `static` and public**
//!    (§14G.2 revision 2). A `secret` collection wrote one public HTML
//!    file per secret, with the secret as the directory name; the build
//!    artefact's file *names* are a declared sink, so the weaker form of
//!    that leak is rejected by the same rule.
//! 3. **The collision check is over rendered URLs** (§14G.2 revision 3),
//!    not over (prefix, arity). `WorkItem is "/work" with slug is Text`
//!    and `WorkFeed is "/work/feed"` differ in both prefix and arity and
//!    both render `/work/feed`, so a check on the declaration cannot see
//!    it. Rendering every URL is what makes the route ↔ URL bijection the
//!    design rests on actually exist.
//!
//! Enumerating the URLs here rather than in the emitter is deliberate:
//! the same list is the collision check, the per-page split, the manifest
//! and `Link`'s `href`, and four copies of it would be four chances to
//! disagree about what a program's URLs are.

use std::collections::BTreeMap;

use zdc_hir::{
    DefId, DefKind, ExprId, Hir, HirArmBody, HirElement, HirExprKind, HirNode, HirNodeArmBody,
    HirStmt, Res, RouteTable,
};
use zdc_lexer::Span;

use crate::TypeError;

/// One document a routed program emits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The URL this document is served at, beginning with `/`.
    pub url: String,
    /// The route variant, by its position in the declaration, or `None`
    /// for the not-found document (spec §14G.2: the `None` arm of
    /// `when page`).
    pub variant: Option<usize>,
    /// One value per route parameter, in declaration order.
    pub values: Vec<String>,
    /// A file-name-safe name for this page's module and stylesheet.
    pub slug: String,
}

/// Every document a program emits, and nothing about how it is emitted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Site {
    pub pages: Vec<Page>,
}

/// Check a program's routes and enumerate its pages.
///
/// A program with no `route` has one page and no routing rules to break,
/// which is why an unrouted program is not a special case anywhere below.
pub fn check(hir: &Hir) -> Result<Site, Vec<TypeError>> {
    let mut errors = Vec::new();
    let constants = constants(hir, &mut errors);
    address_is_immutable(hir, &mut errors);

    let Some((_, table)) = &hir.routes else {
        return finish(Site::default(), errors);
    };

    let mut pages: Vec<Page> = Vec::new();
    let mut seen: BTreeMap<String, (String, Span)> = BTreeMap::new();

    for (index, variant) in table.variants.iter().enumerate() {
        let name = route_variant_name(hir, index);
        let mut columns: Vec<Vec<String>> = Vec::new();
        let mut enumerable = true;

        for param in &variant.params {
            let Some(source) = param.enumerated_in else {
                enumerable = false;
                continue;
            };
            match enumerated_values(hir, source, &constants) {
                Ok(values) => columns.push(values),
                Err(error) => {
                    errors.push(error);
                    enumerable = false;
                }
            }
        }

        if !enumerable {
            // §14G.2 revision 3: the collision check cannot be decided for
            // a variant whose values are unknown, so the only sound rule
            // is that it may not share a prefix with anything.
            for (other, sibling) in table.variants.iter().enumerate() {
                if other == index {
                    continue;
                }
                let a = variant.path.trim_end_matches('/');
                let b = sibling.path.trim_end_matches('/');
                if a == b {
                    errors.push(TypeError {
                        message: format!(
                            "`{name}` has a parameter that is not enumerable, and `{}` renders \
                             URLs under the same prefix `{}`. The compiler cannot decide whether \
                             the two collide, so it refuses rather than guessing: give `{name}`'s \
                             parameter an `in` naming a `static` signal, or give one of the two a \
                             different prefix.",
                            route_variant_name(hir, other),
                            variant.path
                        ),
                        span: variant.span,
                        help: None,
                    });
                }
            }
            continue;
        }

        for values in product(&columns) {
            let url = table.url(index, &values);
            if let Some((first, span)) = seen.get(&url) {
                errors.push(TypeError {
                    message: format!(
                        "`{name}` and `{first}` both render the URL `{url}`. A route is a \
                         bijection onto URLs, so two routes rendering one URL would leave the \
                         program with no way to say which one a visitor asked for."
                    ),
                    span: variant.span,
                    help: Some(format!(
                        "`{first}` is declared at byte {} of this program.",
                        span.start
                    )),
                });
                continue;
            }
            seen.insert(url.clone(), (name.clone(), variant.span));
            pages.push(Page {
                slug: slug_of(&url),
                url,
                variant: Some(index),
                values,
            });
        }
    }

    // The not-found document. It is not a declared route and it takes no
    // parameters: it is what `when page` shows for `None`, which is the
    // arm exhaustiveness already forced the program to write.
    pages.push(Page {
        url: "/404".to_string(),
        variant: None,
        values: Vec::new(),
        slug: "not-found".to_string(),
    });

    finish(Site { pages }, errors)
}

fn finish(site: Site, errors: Vec<TypeError>) -> Result<Site, Vec<TypeError>> {
    if errors.is_empty() {
        Ok(site)
    } else {
        Err(errors)
    }
}

/// The declared name of a route variant.
fn route_variant_name(hir: &Hir, index: usize) -> String {
    let Some((def, _)) = &hir.routes else {
        return String::new();
    };
    let DefKind::Choice(choice) = &hir.defs[*def].kind else {
        return String::new();
    };
    choice
        .variants
        .get(index)
        .map(|variant| variant.name.clone())
        .unwrap_or_default()
}

/// The URL path segment a value renders as.
///
/// A value that is not URL-safe is not silently escaped: the URL is part
/// of the program's public interface and a route whose address bar does
/// not match its source is exactly the surprise §7.3 exists to prevent.
pub fn segment_is_safe(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A file-name-safe name for a URL.
fn slug_of(url: &str) -> String {
    let trimmed = url.trim_matches('/');
    if trimmed.is_empty() {
        return "index".to_string();
    }
    trimmed
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Every combination of one value per parameter, in declaration order.
fn product(columns: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = vec![Vec::new()];
    for column in columns {
        let mut next = Vec::with_capacity(rows.len() * column.len());
        for row in &rows {
            for value in column {
                let mut extended = row.clone();
                extended.push(value.clone());
                next.push(extended);
            }
        }
        rows = next;
    }
    rows
}

/// The build-time value of every `static` signal that has one.
///
/// A `static` signal with no build-time value is reported here and left
/// out, so a route reading it says "this is not enumerable" rather than
/// repeating the same complaint.
fn constants(hir: &Hir, errors: &mut Vec<TypeError>) -> BTreeMap<DefId, Constant> {
    let mut known: BTreeMap<DefId, Constant> = BTreeMap::new();
    // One pass in declaration order, then repeated until nothing new is
    // learned: a `static` signal may be written above the one it reads.
    loop {
        let mut learned = false;
        for (id, def) in hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            if signal.placement != zdc_ast::Placement::Static || known.contains_key(&id) {
                continue;
            }
            if let Some(value) = fold(hir, signal.init, &known) {
                known.insert(id, value);
                learned = true;
            }
        }
        if !learned {
            break;
        }
    }

    for (id, def) in hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        if signal.placement != zdc_ast::Placement::Static || known.contains_key(&id) {
            continue;
        }
        errors.push(TypeError {
            message: format!(
                "`{}` is `static`, so it is evaluated once on the build host and inlined into \
                 every page that reads it — but this initialiser is not something the build host \
                 can evaluate. Write a literal, or a list or map of literals, or read another \
                 `static` signal.",
                def.name
            ),
            span: def.span,
            help: Some(
                "Reading a file at build time needs the foreign function interface (§14E), which \
                 this compiler does not have yet."
                    .to_string(),
            ),
        });
    }
    known
}

/// A build-time value.
#[derive(Debug, Clone, PartialEq)]
enum Constant {
    Text(String),
    Number(f64),
    Truth(bool),
    List(Vec<Constant>),
}

impl Constant {
    /// What this value renders as in a URL, if it can appear in one.
    fn as_segment(&self) -> Option<String> {
        match self {
            Constant::Text(text) => Some(text.clone()),
            Constant::Number(n) => Some(crate::routing::number_to_text(*n)),
            Constant::Truth(_) | Constant::List(_) => None,
        }
    }
}

/// The same rendering JavaScript's `String(n)` gives, for the whole
/// numbers a route parameter can be.
fn number_to_text(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn fold(hir: &Hir, id: ExprId, known: &BTreeMap<DefId, Constant>) -> Option<Constant> {
    match &hir.exprs[id].kind {
        HirExprKind::Text(text) => Some(Constant::Text(text.clone())),
        HirExprKind::Number(n) => Some(Constant::Number(*n)),
        HirExprKind::Truth(truth) => Some(Constant::Truth(*truth)),
        HirExprKind::List(items) => items
            .iter()
            .map(|item| fold(hir, *item, known))
            .collect::<Option<Vec<_>>>()
            .map(Constant::List),
        HirExprKind::Ref(Res::Def(def)) => known.get(def).cloned(),
        HirExprKind::Empty
        | HirExprKind::Map(_)
        | HirExprKind::Ref(_)
        | HirExprKind::Call { .. }
        | HirExprKind::Environment(_)
        | HirExprKind::Address
        | HirExprKind::Unary { .. }
        | HirExprKind::Binary { .. }
        | HirExprKind::Field { .. }
        | HirExprKind::Index { .. } => None,
    }
}

/// The values a route parameter ranges over, or why it does not have any.
fn enumerated_values(
    hir: &Hir,
    source: DefId,
    known: &BTreeMap<DefId, Constant>,
) -> Result<Vec<String>, TypeError> {
    let def = &hir.defs[source];
    let DefKind::Signal(signal) = &def.kind else {
        return Err(TypeError {
            message: format!(
                "`{}` is not a `state` declaration, and the `in` of a route parameter names a \
                 `static` signal holding every value the parameter ranges over.",
                def.name
            ),
            span: def.span,
            help: None,
        });
    };

    // §14G.2 revision 2, the first half. The build writes one public file
    // per enumerated value, so a `secret` collection publishes its
    // secrets as directory names.
    if signal.secret {
        return Err(TypeError {
            message: format!(
                "`{}` is `secret`, and a route parameter enumerated over it would write one \
                 public file per secret value — with the secret as the URL. A build artefact's \
                 file names are a public sink, so this is a leak whether or not the page shows \
                 the value.",
                def.name
            ),
            span: def.span,
            help: Some(
                "Derive a public list from it and enumerate over that, or drop the `in` and \
                 accept the parameter as untrusted (spec §14G.2 revision 2)."
                    .to_string(),
            ),
        });
    }

    // §14G.2 revision 2, the second half, and §14G.7.5: enumerability
    // composes because routing reads the placement and `static` supplies
    // it. There is no route manifest and no `getStaticPaths`.
    if signal.placement != zdc_ast::Placement::Static {
        return Err(TypeError {
            message: format!(
                "`{}` is `{}`-placed, and the `in` of a route parameter must be `static`. The \
                 build has to know every URL before it writes any of them, and a value that is \
                 not known at build time cannot be enumerated.",
                def.name,
                signal.placement.word()
            ),
            span: def.span,
            help: None,
        });
    }

    let Some(Constant::List(items)) = known.get(&source) else {
        return Err(TypeError {
            message: format!(
                "`{}` is not a list of values the build host can evaluate, so a route parameter \
                 cannot be enumerated over it.",
                def.name
            ),
            span: def.span,
            help: None,
        });
    };

    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let Some(segment) = item.as_segment() else {
            return Err(TypeError {
                message: format!(
                    "`{}` holds a value that cannot appear in a URL. A route parameter ranges \
                     over `Text` or `Whole` values.",
                    def.name
                ),
                span: def.span,
                help: None,
            });
        };
        if !segment_is_safe(&segment) {
            return Err(TypeError {
                message: format!(
                    "`{}` holds `{segment}`, which is not a URL path segment. A route parameter's \
                     values are written into the address bar as they are, never escaped, because \
                     a URL that does not match its source is a surprise the program cannot see.",
                    def.name
                ),
                span: def.span,
                help: Some(
                    "Letters, digits, `-`, `_` and `.` are the segment characters.".to_string(),
                ),
            });
        }
        values.push(segment);
    }
    Ok(values)
}

/// §14G.2 revision 1: a signal initialised `starting address` is
/// immutable, exactly as `static` state is.
///
/// This is the load-bearing rule. It is what makes per-URL constant
/// folding ordinary constant propagation over an immutable signal rather
/// than a new evaluation mode, and it is what leaves `Link` as the only
/// navigation — so every navigation is a real anchor.
fn address_is_immutable(hir: &Hir, errors: &mut Vec<TypeError>) {
    let mut from_address: BTreeMap<DefId, Span> = BTreeMap::new();
    for (id, def) in hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        if matches!(hir.exprs[signal.init].kind, HirExprKind::Address) {
            from_address.insert(id, def.span);
        }
    }
    if from_address.is_empty() {
        return;
    }

    let mut writes: Vec<(DefId, Span)> = Vec::new();
    for (_, def) in hir.defs.iter() {
        match &def.kind {
            DefKind::Function(function) => block_writes(hir, function.body, &mut writes),
            DefKind::View(view) => node_writes(hir, &view.nodes, &mut writes),
            DefKind::Signal(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => {}
        }
    }

    for (target, span) in writes {
        let Some(_) = from_address.get(&target) else {
            continue;
        };
        errors.push(TypeError {
            message: format!(
                "`{}` is initialised from `address`, and a signal initialised from `address` is \
                 immutable: the browser writes it once at load and the program never does. \
                 Navigate with a `Link`, which renders a real anchor and starts a fresh program \
                 instance at the target document.",
                hir.defs[target].name
            ),
            span,
            help: Some(
                "Programmatic navigation — navigating from a handler rather than from a link — \
                 is not expressible in v1 (spec §14G.2 revision 1)."
                    .to_string(),
            ),
        });
    }
}

fn node_writes(hir: &Hir, nodes: &[HirNode], out: &mut Vec<(DefId, Span)>) {
    for node in nodes {
        match node {
            HirNode::Element(element) => element_writes(hir, element, out),
            HirNode::Each(each) => node_writes(hir, &each.body, out),
            HirNode::When(when) => {
                for arm in &when.arms {
                    match &arm.body {
                        HirNodeArmBody::Show(element) => element_writes(hir, element, out),
                        HirNodeArmBody::Nodes(nodes) => node_writes(hir, nodes, out),
                    }
                }
            }
            HirNode::If(conditional) => {
                node_writes(hir, &conditional.then, out);
                if let Some(otherwise) = &conditional.otherwise {
                    node_writes(hir, otherwise, out);
                }
            }
            HirNode::Scope(scope) => node_writes(hir, &scope.body, out),
            HirNode::Handler(handler) => block_writes(hir, handler.body, out),
            HirNode::Children(_) => {}
        }
    }
}

/// A two-way `Input` binding is a write too, so binding one to the
/// address signal is the same error by the same rule.
fn element_writes(hir: &Hir, element: &HirElement, out: &mut Vec<(DefId, Span)>) {
    if matches!(element.name.as_str(), "Input" | "Checkbox") {
        if let Some(zdc_hir::HirArg::Positional(expr)) = element.args.first() {
            if let HirExprKind::Ref(Res::Def(def)) = hir.exprs[*expr].kind {
                out.push((def, element.span));
            }
        }
    }
    node_writes(hir, &element.children, out);
}

fn block_writes(hir: &Hir, id: zdc_hir::BlockId, out: &mut Vec<(DefId, Span)>) {
    for stmt in &hir.blocks[id].stmts {
        match stmt {
            HirStmt::Mutation(mutation) => {
                let place = mutation.place();
                if let Res::Def(def) = place.base {
                    out.push((def, place.span));
                }
            }
            HirStmt::When(when) => {
                for arm in &when.arms {
                    if let HirArmBody::Block(block) = arm.body {
                        block_writes(hir, block, out);
                    }
                }
            }
            HirStmt::Each(each) => block_writes(hir, each.body, out),
            HirStmt::If(conditional) => {
                block_writes(hir, conditional.then, out);
                if let Some(otherwise) = conditional.otherwise {
                    block_writes(hir, otherwise, out);
                }
            }
            HirStmt::Pipeline(_) | HirStmt::Give(_) => {}
        }
    }
}

/// Which route variant a `when` arm names, if it names one.
///
/// Variant names are unique across the whole program (name collection
/// rejects a second declaration of one), so a pattern name is enough to
/// answer this without consulting a type.
pub fn route_variant_of(table: &RouteTable, hir: &Hir, pattern: &str) -> Option<usize> {
    let (def, _) = hir.routes.as_ref()?;
    let DefKind::Choice(choice) = &hir.defs[*def].kind else {
        return None;
    };
    let index = choice
        .variants
        .iter()
        .position(|variant| variant.name == pattern)?;
    table.variants.get(index).map(|_| index)
}
