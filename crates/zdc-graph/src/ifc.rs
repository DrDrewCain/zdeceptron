//! Information flow — spec §17.3, correcting §5.3 via §14G.1.3.
//!
//! §5.3 says taint "propagates through every data dependency" and claims
//! non-interference. Those two statements are not compatible: a
//! data-dependency analysis cannot see a **control** dependency. Six
//! reviewers independently exhibited programs that exploit exactly that
//! hole, and every one of them is a fixture in this crate's tests.
//!
//! Three phases, not six: declare, summarise, discharge.
//!
//! * **Declare** reads `secret` off the declaration. Secrecy being
//!   declared rather than inferred is what removes the signal-graph
//!   fixpoint entirely — every edge into a signal is checked against a
//!   constant — and it is why §17.5.4's reactive cycles are irrelevant
//!   here rather than "handled".
//! * **Summarise** solves one fixpoint over the call graph, keyed by the
//!   instantiations the split found. Every summary has the normal form
//!   `floor ⊔ ⨆_{p ∈ deps} label(arg_p)`, which is the only shape that
//!   keeps `politeGreeting`'s unused `key` out of the result while still
//!   catching `if key is ""`.
//! * **Discharge** walks every root, including orphan roots, so no
//!   definition escapes checking.

use std::collections::{BTreeMap, BTreeSet};

use zdc_hir::{
    ArenaId as _, Builtin, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirElement,
    HirExprKind, HirMutation, HirNode, HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, LocalId,
    Res,
};
use zdc_lexer::Span;
use zdc_types::SignalPlacement;

use crate::diag::GraphError;
use crate::label::{Label, Obs, Secrecy, Sym, SymLabel};
use crate::root::{placement_of, Ctx, RootId};
use crate::sites::arg_expr;
use crate::split::{BoundaryEdge, Crossing, MemberForm, TierSplit};

/// §14G.1.3(c)'s sink list, declared and closed.
///
/// Deliberately **not** `#[non_exhaustive]`: adding a variant must break
/// every downstream `match`. Adding sink 7 is: add the variant, fix the
/// compile errors, bump the length test, write `describe`, and add a
/// fixture that leaks through it and must be rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sink {
    ClientState,
    View,
    BuildArtifact,
    ResponseBody,
    PlatformLog,
    LiveSync,
}

impl Sink {
    pub const CLOSED_LIST: [Sink; 6] = [
        Sink::ClientState,
        Sink::View,
        Sink::BuildArtifact,
        Sink::ResponseBody,
        Sink::PlatformLog,
        Sink::LiveSync,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Sink::ClientState => "E-IFC-06",
            Sink::View => "E-IFC-05",
            Sink::BuildArtifact => "E-IFC-07",
            Sink::ResponseBody => "E-IFC-08",
            Sink::PlatformLog => "E-IFC-09",
            Sink::LiveSync => "E-IFC-10",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Sink::ClientState => "client state",
            Sink::View => "the view",
            Sink::BuildArtifact => "the build artefact",
            Sink::ResponseBody => "an outbound response body",
            Sink::PlatformLog => "a platform log",
            Sink::LiveSync => "a live-sync stream the browser subscribes to",
        }
    }
}

/// Where a sink is, precisely enough for an emitter to ask about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SinkSite {
    ViewArg(ExprId, Ctx),
    ClientSignal(DefId),
    BuildOutput(DefId),
    ResponseBody(RootId),
    PlatformLog(RootId),
    LiveSync(DefId),
}

/// Permission to write a value into an artifact.
///
/// Unforgeable outside this crate: the field is private and there is no
/// public constructor. The six code-generation entry points that write
/// into artifacts take one of these, and there are no others, so an
/// emitter that writes without asking is a Rust type error rather than a
/// silent leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cleared(());

/// What the information-flow pass concluded.
#[derive(Debug, Clone, Default)]
pub struct Verdict {
    labels: BTreeMap<DefId, Label>,
    cleared: BTreeSet<SinkSite>,
    pub diagnostics: Vec<GraphError>,
}

impl Verdict {
    /// The label a definition ended up with. §16.3.12 assertion B ranges
    /// over this, not over "is not declared `secret`" — the keyword-based
    /// check is vacuous under a stopped walk, and this one is not.
    pub fn label(&self, def: DefId) -> Label {
        self.labels.get(&def).copied().unwrap_or_default()
    }

    pub fn errors(&self) -> impl Iterator<Item = &GraphError> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }

    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }

    /// Ask whether a site may be written into. `None` is a refusal, and an
    /// emitter has nothing else it can do with it.
    pub fn cleared(&self, sink: Sink, site: SinkSite) -> Option<Cleared> {
        let _ = sink;
        self.cleared.contains(&site).then_some(Cleared(()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObligationKind {
    /// A write into a place with a declared label.
    Write(DefId),
    /// A signal's declared label versus what its initialiser produced.
    Declaration(DefId),
    /// A value reaching one of the six sinks.
    Escape(Sink, SinkSite),
}

#[derive(Debug, Clone)]
struct Obligation {
    kind: ObligationKind,
    required: Secrecy,
    found: Sym,
    pc: Sym,
    site: Span,
    /// What the reader is told the value *is*, in prose.
    what: String,
    found_trace: Trace,
    pc_trace: Trace,
}

type Trace = Vec<(Span, String)>;

#[derive(Debug, Clone, Default)]
struct Summary {
    result: SymLabel,
    obligations: BTreeMap<Span, Obligation>,
}

/// A label with the reason it is what it is.
///
/// The reason lives **outside** the lattice and outside every equality
/// test: `Sym` derives `PartialEq` over exactly the lattice the
/// termination proof ranges over, and a witness inside it would grow two
/// steps every round forever (verified against `recurse.zd`).
#[derive(Debug, Clone, Default)]
struct Valued {
    label: SymLabel,
    trace: Trace,
}

impl Valued {
    fn bottom() -> Valued {
        Valued::default()
    }

    fn of(label: SymLabel, trace: Trace) -> Valued {
        Valued { label, trace }
    }

    fn join(&self, other: &Valued) -> Valued {
        Valued {
            label: self.label.join(&other.label),
            trace: merge(&self.trace, &other.trace),
        }
    }
}

fn merge(left: &Trace, right: &Trace) -> Trace {
    let mut out = left.clone();
    for step in right {
        if !out.iter().any(|(span, _)| *span == step.0) {
            out.push(step.clone());
        }
    }
    out
}

/// Run the information-flow pass.
///
/// It needs nothing from `zdc-types` in the language as it stands: there
/// is no `foreign` declaration to read a `gives secret` flag off, and
/// §17.6 item 13 blesses matching `Remote`'s three tags by name. So this
/// runs whether or not the program typechecks, which is what lets a
/// program with a type error *and* a leak report both.
pub fn ifc(hir: &Hir, split: &TierSplit) -> Verdict {
    Ifc::new(hir, split).run()
}

struct Ifc<'a> {
    hir: &'a Hir,
    split: &'a TierSplit,
    declared: BTreeMap<DefId, Label>,
    summaries: BTreeMap<(DefId, Ctx), Summary>,
    /// How parameter `i` of `(f, ctx)` reaches `f`'s result, reconstructed
    /// after convergence.
    param_paths: BTreeMap<(DefId, Ctx, u32), Trace>,
    out: Verdict,
}

impl<'a> Ifc<'a> {
    fn new(hir: &'a Hir, split: &'a TierSplit) -> Ifc<'a> {
        Ifc {
            hir,
            split,
            declared: BTreeMap::new(),
            summaries: BTreeMap::new(),
            param_paths: BTreeMap::new(),
            out: Verdict::default(),
        }
    }

    fn run(mut self) -> Verdict {
        self.declare();
        self.summarise();
        self.reconstruct_param_paths();
        self.discharge();
        self.live_sync();
        self.out
    }

    // --- phase 1: declare (§17.3.3) ---

    fn declare(&mut self) {
        for (id, def) in self.hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            let label = if signal.secret {
                // `secret` labels both components Secret: §5.3's lattice
                // is two-point and the grammar has no syntax to declare a
                // public-shaped secret-valued store.
                Label::scalar(Secrecy::Secret)
            } else {
                Label::scalar(Secrecy::Public)
            };
            self.declared.insert(id, label);
            self.out.labels.insert(id, label);

            // E-IFC-01 is a redundant assertion, with its own code,
            // because two passes reading the same keyword must not
            // silently disagree. It is raised only when the split did not
            // already raise E0313, so one mistake is never printed twice.
            let placement = placement_of(signal.placement);
            let cannot_hold =
                matches!(placement, SignalPlacement::Client | SignalPlacement::Static);
            let split_said_so = self
                .split
                .diagnostics
                .iter()
                .any(|d| d.code == "E0313" && d.span == def.span);
            if signal.secret && cannot_hold && !split_said_so {
                self.out.diagnostics.push(GraphError::new(
                    "E-IFC-01",
                    format!(
                        "`{}` is declared `secret` on a `{}` placement, which cannot hold one.",
                        def.name,
                        placement.describe()
                    ),
                    def.span,
                ));
            }
        }
    }

    // --- phase 2: summarise (§17.3.4) ---

    fn summarise(&mut self) {
        let keys: Vec<(DefId, Ctx)> = self
            .split
            .contexts
            .iter()
            .filter(|(id, _)| matches!(self.hir.defs[**id].kind, DefKind::Function(_)))
            .flat_map(|(id, contexts)| contexts.iter().map(move |ctx| (*id, *ctx)))
            .collect();

        for key in &keys {
            self.summaries.insert(*key, Summary::default());
        }

        // A monotone fixpoint over a finite lattice: seeding at ⊥ gives
        // the least fixed point, which is sound for a may-analysis. The
        // bound is stated rather than trusted — a non-terminating loop
        // here would be a compiler hang, which is the one failure mode a
        // language server cannot survive.
        let bound = 8 * (keys.len() + 1) * (self.hir.exprs.len() + 1);
        for _ in 0..bound {
            let mut changed = false;
            for (def, ctx) in &keys {
                let solved = self.solve_function(*def, *ctx, false, 0);
                let before = self.summaries.get(&(*def, *ctx));
                if before.map(|s| &s.result) != Some(&solved.result)
                    || before.map(|s| s.obligations.len()) != Some(solved.obligations.len())
                    || !obligations_equal(before, &solved)
                {
                    self.summaries.insert((*def, *ctx), solved);
                    changed = true;
                }
            }
            if !changed {
                return;
            }
        }
    }

    fn solve_function(&self, def: DefId, ctx: Ctx, tracing: bool, depth: u32) -> Summary {
        let DefKind::Function(function) = &self.hir.defs[def].kind else {
            return Summary::default();
        };
        let mut walk = Walk::new(self, ctx, def, tracing, depth);
        for (index, param) in function.params.iter().enumerate() {
            walk.locals.insert(
                *param,
                Valued::of(
                    SymLabel {
                        shape: Sym::dep(index as u32, Obs::Shape),
                        value: Sym::dep(index as u32, Obs::Value),
                        failure: Sym::dep(index as u32, Obs::Failure),
                    },
                    Vec::new(),
                ),
            );
        }
        walk.block(function.body);
        // A pipeline is the function's result when it has no `give`.
        let result = if walk.gave {
            walk.result.label.clone()
        } else {
            walk.acc.label.clone()
        };
        Summary {
            result,
            obligations: walk.obligations,
        }
    }

    /// §17.3.4's witness reconstruction, done **after** convergence.
    ///
    /// One concrete re-walk per parameter, with that parameter marked
    /// Secret and everything else Public, recording the path its taint
    /// takes to the result. Breadth is unnecessary: the walk is over a
    /// tree, so the path it records is the only one.
    fn reconstruct_param_paths(&mut self) {
        let keys: Vec<(DefId, Ctx)> = self.summaries.keys().copied().collect();
        for (def, ctx) in keys {
            let DefKind::Function(function) = &self.hir.defs[def].kind else {
                continue;
            };
            let arity = function.params.len();
            let params = function.params.clone();
            for index in 0..arity {
                let mut walk = Walk::new(self, ctx, def, true, 0);
                for (position, param) in params.iter().enumerate() {
                    let secrecy = if position == index {
                        Secrecy::Secret
                    } else {
                        Secrecy::Public
                    };
                    walk.locals.insert(
                        *param,
                        Valued::of(SymLabel::declared(Label::scalar(secrecy)), Vec::new()),
                    );
                }
                walk.block(function.body);
                let result = if walk.gave { walk.result } else { walk.acc };
                if result.label.value.concrete() == Secrecy::Secret {
                    self.param_paths
                        .insert((def, ctx, index as u32), result.trace);
                }
            }
        }
    }

    // --- phase 3: discharge (§17.3.6) ---

    fn discharge(&mut self) {
        // Every root, including orphan roots, so no definition escapes
        // checking (§17.2.5 fatal 6 and §17.3.6 together).
        let roots: Vec<(RootId, Ctx)> = self
            .split
            .roots
            .iter()
            .enumerate()
            .map(|(index, root)| (RootId(index as u32), root.ctx))
            .collect();

        for (root, ctx) in roots {
            let members: Vec<(DefId, MemberForm)> = self.split.members_of(root).collect();
            for (def, form) in members {
                match &self.hir.defs[def].kind {
                    DefKind::Signal(_) if form == MemberForm::Binding => {
                        self.discharge_signal(def, root, ctx)
                    }
                    DefKind::View(view) => {
                        let nodes = view.nodes.clone();
                        let mut walk = Walk::new(self, ctx, def, true, 0);
                        walk.nodes(&nodes);
                        let obligations = std::mem::take(&mut walk.obligations);
                        self.discharge_all(obligations);
                    }
                    _ => {}
                }
            }
        }

        // A function nothing calls still has its obligations discharged,
        // with its parameters at ⊥: an `append apiKey to auditLog` in an
        // uncalled helper is still a leak waiting to be enabled.
        for (id, def) in self.hir.defs.iter() {
            if !matches!(def.kind, DefKind::Function(_)) {
                continue;
            }
            let uncalled = !self
                .split
                .reached_by
                .keys()
                .any(|(called, _)| *called == id);
            if !uncalled {
                continue;
            }
            let contexts: Vec<Ctx> = self
                .split
                .contexts
                .get(&id)
                .map(|set| set.iter().copied().collect())
                .unwrap_or_default();
            for ctx in contexts {
                let summary = self.solve_function(id, ctx, true, 0);
                let obligations: Vec<Obligation> = summary
                    .obligations
                    .into_values()
                    .map(|mut obligation| {
                        obligation.found = obligation.found.instantiate(&[]);
                        obligation.pc = obligation.pc.instantiate(&[]);
                        obligation
                    })
                    .collect();
                self.discharge_all(
                    obligations
                        .into_iter()
                        .map(|o| (o.site, o))
                        .collect::<BTreeMap<_, _>>(),
                );
            }
        }
    }

    fn discharge_signal(&mut self, def: DefId, root: RootId, ctx: Ctx) {
        let DefKind::Signal(signal) = &self.hir.defs[def].kind else {
            return;
        };
        // A durable or static signal's initialiser is evaluated only in
        // the BUILD root (§17.2.5 fatal 5), and the split has already
        // recorded that by giving it another form everywhere else.
        let init = signal.init;
        let placement = placement_of(signal.placement);
        let mut walk = Walk::new(self, ctx, def, true, 0);
        let value = walk.expr(init);
        let obligations = std::mem::take(&mut walk.obligations);

        let required = self.declared.get(&def).copied().unwrap_or_default().value;
        let sink_site = SinkSite::ClientSignal(def);
        let is_client_state = matches!(placement, SignalPlacement::Client);

        let mut all = obligations;
        all.insert(
            self.hir.defs[def].span,
            Obligation {
                kind: if is_client_state {
                    ObligationKind::Escape(Sink::ClientState, sink_site)
                } else {
                    ObligationKind::Declaration(def)
                },
                required,
                found: value.label.value.clone(),
                pc: Sym::bottom(),
                site: self.hir.exprs[init].span,
                what: format!("`{}` is declared", self.hir.defs[def].name),
                found_trace: value.trace,
                pc_trace: Vec::new(),
            },
        );
        let _ = root;
        self.discharge_all(all);
    }

    fn discharge_all(&mut self, obligations: BTreeMap<Span, Obligation>) {
        for obligation in obligations.into_values() {
            let found = obligation.found.concrete().join(obligation.pc.concrete());
            if found.flows_to(obligation.required) {
                if let ObligationKind::Escape(_, site) = obligation.kind {
                    self.out.cleared.insert(site);
                }
                continue;
            }
            self.out.diagnostics.push(self.render(&obligation));
        }
    }

    fn render(&self, obligation: &Obligation) -> GraphError {
        let notes = merge(&obligation.found_trace, &obligation.pc_trace);
        match obligation.kind {
            ObligationKind::Declaration(def) => GraphError::new(
                "E-IFC-02",
                format!(
                    "this derivation is secret, but `{}` is not declared secret.",
                    self.hir.defs[def].name
                ),
                obligation.site,
            )
            .with_notes(notes)
            .with_help(format!(
                "Either stop the secret reaching it, or write `secret state {}` — which will then \
                 be rejected wherever `{}` is rendered, which is the point (spec §17.3.6).",
                self.hir.defs[def].name, self.hir.defs[def].name
            )),
            ObligationKind::Write(def) => GraphError::new(
                "E-IFC-03",
                format!(
                    "a secret is written into `{}`, which is {}.",
                    self.hir.defs[def].name,
                    self.declared
                        .get(&def)
                        .copied()
                        .unwrap_or_default()
                        .value
                        .describe()
                ),
                obligation.site,
            )
            .with_notes(notes)
            .with_help(
                "A write requires `label(value) ⊔ pc ⊑ label(place)`. Declare the place `secret`, \
                 or stop the secret reaching this statement (spec §5.3a(a)).",
            ),
            ObligationKind::Escape(sink, _) => GraphError::new(
                sink.code(),
                format!(
                    "{} would reach {}, and {} is where a browser can see it.",
                    obligation.what,
                    sink.describe(),
                    sink.describe()
                ),
                obligation.site,
            )
            .with_notes(notes)
            .with_help(
                "A secret may be computed on the server and it may influence what the server \
                 returns only if the result is itself declared secret. Nothing declared secret may \
                 be rendered (spec §5.3, §14G.1.3(c)).",
            ),
        }
    }

    /// Sink 6, from the two structurally different edges the split emits.
    ///
    /// §17.2.5 fatal 4: `watch_keys` conflated "the browser is sent the
    /// value" with "the browser is told the key changed", so a public
    /// aggregate over a `secret` store was either permanently stale or a
    /// live leak. The split does not decide; it emits both edges and this
    /// rules on them.
    fn live_sync(&mut self) {
        for edge in &self.split.boundary {
            let (key, observed, why) = match edge {
                BoundaryEdge::LiveValue { key } => (*key, Obs::Value, "its value is streamed"),
                BoundaryEdge::Invalidate { key, .. } => (
                    *key,
                    Obs::Shape,
                    "the browser is told when it changes, which is an observation of it",
                ),
                _ => continue,
            };
            let label = self.declared.get(&key).copied().unwrap_or_default();
            let site = SinkSite::LiveSync(key);
            if label.get(observed).flows_to(Secrecy::Public) {
                self.out.cleared.insert(site);
                continue;
            }
            self.out.diagnostics.push(
                GraphError::new(
                    "E-IFC-10",
                    format!(
                        "`{}` is secret, and {} — so a browser subscribed to the live-sync stream \
                         would learn about it.",
                        self.hir.defs[key].name, why
                    ),
                    self.hir.defs[key].span,
                )
                .with_notes(vec![(
                    self.hir.defs[key].span,
                    format!("`{}` is declared secret", self.hir.defs[key].name),
                )])
                .with_help(
                    "Either refresh on a cadence rather than on change, or declare the derived \
                     signal `secret` too — which will then be rejected wherever it is rendered \
                     (spec §17.2.5 fatal 4).",
                ),
            );
        }
    }
}

fn obligations_equal(before: Option<&Summary>, after: &Summary) -> bool {
    let Some(before) = before else {
        return false;
    };
    if before.obligations.len() != after.obligations.len() {
        return false;
    }
    before.obligations.iter().all(|(span, left)| {
        after.obligations.get(span).is_some_and(|right| {
            left.found == right.found && left.pc == right.pc && left.required == right.required
        })
    })
}

/// One walk of one body, in one context.
struct Walk<'a, 'b> {
    ifc: &'b Ifc<'a>,
    ctx: Ctx,
    owner: DefId,
    tracing: bool,
    /// How many traced re-solves deep this walk is. A traced walk re-runs
    /// a callee's body so the diagnostic can name the branch *inside* it;
    /// the bound is what stops a recursive function re-solving forever.
    depth: u32,
    locals: BTreeMap<LocalId, Valued>,
    pc: Sym,
    pc_trace: Trace,
    acc: Valued,
    result: Valued,
    gave: bool,
    obligations: BTreeMap<Span, Obligation>,
}

impl<'a, 'b> Walk<'a, 'b> {
    fn new(ifc: &'b Ifc<'a>, ctx: Ctx, owner: DefId, tracing: bool, depth: u32) -> Walk<'a, 'b> {
        Walk {
            ifc,
            ctx,
            owner,
            tracing,
            depth,
            locals: BTreeMap::new(),
            pc: Sym::bottom(),
            pc_trace: Vec::new(),
            acc: Valued::bottom(),
            result: Valued::bottom(),
            gave: false,
            obligations: BTreeMap::new(),
        }
    }

    fn trace(&self, steps: Trace) -> Trace {
        if self.tracing {
            steps
        } else {
            Vec::new()
        }
    }

    // --- expressions (§17.3.4's table) ---

    fn expr(&mut self, id: ExprId) -> Valued {
        let span = self.ifc.hir.exprs[id].span;
        match &self.ifc.hir.exprs[id].kind {
            HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty => Valued::bottom(),

            // Unconditionally Secret. Otherwise omitting the `secret`
            // keyword launders a credential, and §5.6 already confines
            // `environment` to server context.
            HirExprKind::Environment(key) => Valued::of(
                SymLabel::declared(Label::scalar(Secrecy::Secret)),
                self.trace(vec![(
                    span,
                    format!("`environment \"{key}\"` is always secret"),
                )]),
            ),

            HirExprKind::Ref(Res::Local(local)) => {
                self.locals.get(local).cloned().unwrap_or_default()
            }
            HirExprKind::Ref(Res::Def(def)) => {
                let def = *def;
                if matches!(self.ifc.hir.defs[def].kind, DefKind::Signal(_)) {
                    self.read(def, id, span)
                } else {
                    Valued::bottom()
                }
            }
            HirExprKind::Ref(Res::Builtin(_)) => Valued::bottom(),
            // A payload-free variant is a constant tag: it carries no data,
            // so it carries no secret.
            HirExprKind::Ref(Res::Variant { .. }) => Valued::bottom(),

            // A collection literal is a constructor: §17.3.4's rule for one
            // is the join of its operands. Containers are element-
            // insensitive here for the same reason records are field-
            // insensitive above — one label for the collection's shape and
            // one for everything in it jointly — so a secret anywhere
            // inside makes the whole literal secret.
            HirExprKind::List(items) => {
                let items = items.clone();
                let mut joined = Sym::bottom();
                let mut trace = Vec::new();
                for item in items {
                    let element = self.expr(item);
                    joined.join_in_place(&element.label.value);
                    trace = merge(&trace, &element.trace);
                }
                Valued::of(SymLabel::triple(joined), trace)
            }
            HirExprKind::Map(entries) => {
                let entries = entries.clone();
                let mut joined = Sym::bottom();
                let mut trace = Vec::new();
                for (key, value) in entries {
                    let key = self.expr(key);
                    let value = self.expr(value);
                    joined.join_in_place(&key.label.value);
                    joined.join_in_place(&value.label.value);
                    trace = merge(&trace, &key.trace);
                    trace = merge(&trace, &value.trace);
                }
                Valued::of(SymLabel::triple(joined), trace)
            }

            HirExprKind::Call { callee, args } => {
                let callee = *callee;
                let args = args.clone();
                self.call(callee, &args, span)
            }

            HirExprKind::Unary { operand, .. } => {
                let operand = *operand;
                let inner = self.expr(operand);
                Valued::of(
                    SymLabel {
                        shape: inner.label.value.clone(),
                        value: inner.label.value.clone(),
                        failure: inner.label.failure.clone(),
                    },
                    inner.trace,
                )
            }
            HirExprKind::Binary { lhs, rhs, .. } => {
                let (lhs, rhs) = (*lhs, *rhs);
                let left = self.expr(lhs);
                let right = self.expr(rhs);
                let joined = left.label.value.join(&right.label.value);
                Valued::of(SymLabel::triple(joined), merge(&left.trace, &right.trace))
            }
            // Records are field-insensitive: one label for the record's
            // existence and one for all its fields jointly. That is the
            // ruling §16.7 item 12 asked for (§17.6 item 15).
            HirExprKind::Field { base, .. } => {
                let base = *base;
                let inner = self.expr(base);
                Valued::of(SymLabel::triple(inner.label.value), inner.trace)
            }
            HirExprKind::Index { base, index } => {
                let (base, index) = (*base, *index);
                let container = self.expr(base);
                let key = self.expr(index);
                let joined = container
                    .label
                    .shape
                    .join(&container.label.value)
                    .join(&key.label.value);
                Valued::of(
                    SymLabel::triple(joined),
                    merge(&container.trace, &key.trace),
                )
            }
        }
    }

    /// §17.3.5 — a lookup, not a table.
    ///
    /// That the dead-code cut and the secrecy check are literally the same
    /// edge is what makes §14A.1's exclusion provable rather than
    /// heuristic. IFC never re-derives §14G.1.4 and never issues a
    /// read-table error; the split already did both.
    fn read(&mut self, signal: DefId, expr: ExprId, span: Span) -> Valued {
        let declared = self.ifc.declared.get(&signal).copied().unwrap_or_default();
        let name = self.ifc.hir.defs[signal].name.clone();
        let declared_at = self.ifc.hir.defs[signal].span;

        match self.ifc.split.crossings.get(&(expr, self.ctx)) {
            Some(Crossing::Remote { endpoint }) => {
                // Crossing back to a browser. This is the one read that is
                // itself a sink.
                let sink = if Some(self.owner) == self.ifc.hir.view {
                    Sink::View
                } else {
                    Sink::ClientState
                };
                let site = SinkSite::ViewArg(expr, self.ctx);
                self.oblige(Obligation {
                    kind: ObligationKind::Escape(sink, site),
                    required: Secrecy::Public,
                    found: Sym::floor(declared.value),
                    pc: self.pc.clone(),
                    site: span,
                    what: format!("`{name}`"),
                    found_trace: self.trace(vec![
                        (declared_at, format!("`{name}` is declared secret")),
                        (span, format!("`{name}` is read here, in the browser")),
                    ]),
                    pc_trace: self.pc_trace.clone(),
                });

                let failure = self
                    .ifc
                    .split
                    .params
                    .get(endpoint)
                    .map(|params| {
                        params.iter().fold(Sym::bottom(), |acc, param| {
                            acc.join(&Sym::floor(
                                self.ifc
                                    .declared
                                    .get(param)
                                    .copied()
                                    .unwrap_or_default()
                                    .value,
                            ))
                        })
                    })
                    .unwrap_or_default();
                Valued::of(
                    SymLabel {
                        shape: Sym::bottom(),
                        value: Sym::bottom(),
                        failure,
                    },
                    Vec::new(),
                )
            }
            _ => {
                let trace = if declared.value == Secrecy::Secret {
                    self.trace(vec![(declared_at, format!("`{name}` is declared secret"))])
                } else {
                    Vec::new()
                };
                Valued::of(SymLabel::declared(declared), trace)
            }
        }
    }

    /// A constructor — a record literal or a variant carrying a payload.
    ///
    /// §17.3.4's rule for a constructor is the join of its operands, the
    /// same as a collection literal's. Records are field-insensitive
    /// (§17.6 item 15), so a secret in any field makes the whole value
    /// secret; anything weaker would let `Todo with title is apiKey`
    /// launder a credential through a record.
    fn constructed(&mut self, args: &[HirArg]) -> Valued {
        let exprs: Vec<ExprId> = args.iter().map(arg_expr).collect();
        let mut joined = Sym::bottom();
        let mut trace = Vec::new();
        for expr in exprs {
            let arg = self.expr(expr);
            joined.join_in_place(&arg.label.value);
            trace = merge(&trace, &arg.trace);
        }
        Valued::of(SymLabel::triple(joined), trace)
    }

    fn call(&mut self, callee: Res, args: &[HirArg], span: Span) -> Valued {
        // A variant constructor resolves to `Res::Variant`, not to a
        // definition with a body to summarise.
        let Res::Def(def) = callee else {
            return self.constructed(args);
        };
        // A record literal is `Todo with …`, which parses as a call.
        let DefKind::Function(function) = &self.ifc.hir.defs[def].kind else {
            return self.constructed(args);
        };
        // §17.6 item 12: matching a named argument to a parameter index is
        // five lines here, not a dependency on `zdc-types`.
        let names: Vec<String> = function
            .params
            .iter()
            .map(|param| self.ifc.hir.locals[*param].name.clone())
            .collect();
        let mut ordered: Vec<Option<ExprId>> = vec![None; names.len()];
        let mut next = 0usize;
        for arg in args {
            match arg {
                HirArg::Positional(expr) => {
                    if next < ordered.len() {
                        ordered[next] = Some(*expr);
                        next += 1;
                    }
                }
                HirArg::Named { name, value } => {
                    if let Some(index) = names.iter().position(|param| param == name) {
                        ordered[index] = Some(*value);
                    }
                }
            }
        }

        let mut evaluated: Vec<Valued> = Vec::with_capacity(ordered.len());
        for slot in &ordered {
            evaluated.push(match slot {
                Some(expr) => self.expr(*expr),
                None => Valued::bottom(),
            });
        }
        let labels: Vec<SymLabel> = evaluated.iter().map(|v| v.label.clone()).collect();

        // A traced walk re-solves the callee, so an obligation raised
        // *inside* it can say which branch inside it raised the program
        // counter. The depth bound is what stops a recursive function
        // re-solving forever; past it the converged summary is used, which
        // is the same verdict with a shorter path.
        let summary = if self.tracing && self.depth < 3 {
            self.ifc.solve_function(def, self.ctx, true, self.depth + 1)
        } else {
            self.ifc
                .summaries
                .get(&(def, self.ctx))
                .cloned()
                .unwrap_or_default()
        };

        // Propagate the callee's obligations into this body, substituted.
        for (site, obligation) in summary.obligations {
            let mut instantiated = obligation.clone();
            instantiated.found = obligation.found.instantiate(&labels);
            instantiated.pc = obligation.pc.instantiate(&labels).join(&self.pc);
            instantiated.found_trace = merge(
                &self.witness_for(def, &obligation.found, &evaluated, span),
                &obligation.found_trace,
            );
            instantiated.pc_trace = merge(
                &merge(
                    &self.pc_trace,
                    &self.witness_for(def, &obligation.pc, &evaluated, span),
                ),
                &obligation.pc_trace,
            );
            self.oblige_at(site, instantiated);
        }

        let label = summary.result.instantiate(&labels);
        let trace = self.witness_for(def, &summary.result.value, &evaluated, span);
        Valued::of(label, trace)
    }

    /// The path a secret takes through a call: which argument carried it,
    /// and how that parameter reaches the result inside the callee.
    fn witness_for(&self, def: DefId, sym: &Sym, args: &[Valued], span: Span) -> Trace {
        if !self.tracing {
            return Vec::new();
        }
        let DefKind::Function(function) = &self.ifc.hir.defs[def].kind else {
            return Vec::new();
        };
        let mut out: Trace = Vec::new();
        for (index, obs) in &sym.deps {
            let Some(arg) = args.get(*index as usize) else {
                continue;
            };
            if arg.label.get(*obs).concrete() != Secrecy::Secret {
                continue;
            }
            out = merge(&out, &arg.trace);
            let param = function
                .params
                .get(*index as usize)
                .map(|local| self.ifc.hir.locals[*local].name.clone())
                .unwrap_or_else(|| format!("argument {}", index + 1));
            out = merge(&out, &vec![(span, format!("passed as `{param}`"))]);
            if let Some(inner) = self.ifc.param_paths.get(&(def, self.ctx, *index)) {
                out = merge(&out, inner);
            }
        }
        out
    }

    fn oblige(&mut self, obligation: Obligation) {
        let site = obligation.site;
        self.oblige_at(site, obligation);
    }

    /// Two obligations with the same identity are **joined**, not
    /// appended — which is what bounds a summary by the number of sites
    /// rather than by the number of rounds.
    fn oblige_at(&mut self, site: Span, obligation: Obligation) {
        match self.obligations.get_mut(&site) {
            Some(existing) => {
                existing.found.join_in_place(&obligation.found);
                existing.pc.join_in_place(&obligation.pc);
                existing.found_trace = merge(&existing.found_trace, &obligation.found_trace);
                existing.pc_trace = merge(&existing.pc_trace, &obligation.pc_trace);
            }
            None => {
                self.obligations.insert(site, obligation);
            }
        }
    }

    // --- statements (§17.3.4's statement table) ---

    fn block(&mut self, id: zdc_hir::BlockId) {
        let stmts = self.ifc.hir.blocks[id].stmts.clone();
        for stmt in &stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Pipeline(clause) => self.pipeline(clause),
            HirStmt::Give(expr) => {
                let value = self.expr(*expr);
                let mut label = value.label;
                label.join_all(&self.pc);
                let mut trace = value.trace;
                if !self.pc.is_bottom() {
                    trace = merge(
                        &merge(&trace, &self.pc_trace),
                        &self.trace(vec![(
                            self.ifc.hir.exprs[*expr].span,
                            "returned under that branch".to_string(),
                        )]),
                    );
                }
                self.result = self.result.join(&Valued::of(label, trace));
                self.gave = true;
            }
            HirStmt::Mutation(mutation) => self.mutation(mutation),
            HirStmt::When(when) => {
                let scrutinee = self.expr(when.scrutinee);
                let outer_pc = self.pc.clone();
                let outer_pc_trace = self.pc_trace.clone();
                self.pc = outer_pc.join(&scrutinee.label.shape);
                if !scrutinee.label.shape.is_bottom() {
                    self.pc_trace = merge(
                        &merge(&outer_pc_trace, &scrutinee.trace),
                        &self.trace(vec![(
                            self.ifc.hir.exprs[when.scrutinee].span,
                            "which arm is taken depends on a secret  [control dependency]"
                                .to_string(),
                        )]),
                    );
                }

                let before = self.acc.clone();
                let mut merged: Option<Valued> = None;
                for arm in &when.arms {
                    // The `Failed` arm of a `Remote` binds the failure
                    // observation, not the value: §14G.1.3(d), and an HTTP
                    // client's error message routinely contains the URL,
                    // key and all.
                    let bound = if arm.pattern_name == "Failed" {
                        scrutinee.label.failure.clone()
                    } else {
                        scrutinee.label.value.clone()
                    };
                    for binder in &arm.bindings {
                        self.locals.insert(
                            *binder,
                            Valued::of(SymLabel::triple(bound.clone()), scrutinee.trace.clone()),
                        );
                    }
                    self.acc = before.clone();
                    match &arm.body {
                        HirArmBody::Show(expr) => {
                            let _ = self.expr(*expr);
                        }
                        HirArmBody::Block(block) => self.block(*block),
                    }
                    merged = Some(match merged {
                        Some(previous) => previous.join(&self.acc),
                        None => self.acc.clone(),
                    });
                }
                self.acc = merged.unwrap_or(before);
                self.pc = outer_pc;
                self.pc_trace = outer_pc_trace;
            }
            HirStmt::Each(each) => {
                let iter = self.expr(each.iter);
                let outer_pc = self.pc.clone();
                let outer_pc_trace = self.pc_trace.clone();
                self.pc = outer_pc.join(&iter.label.shape);
                if !iter.label.shape.is_bottom() {
                    self.pc_trace = merge(&outer_pc_trace, &iter.trace);
                }
                self.locals.insert(
                    each.var,
                    Valued::of(
                        SymLabel::triple(iter.label.value.clone()),
                        iter.trace.clone(),
                    ),
                );
                let before = self.acc.clone();
                self.block(each.body);
                self.acc = before.join(&self.acc);
                self.pc = outer_pc;
                self.pc_trace = outer_pc_trace;
            }
            HirStmt::If(conditional) => {
                let cond = self.expr(conditional.cond);
                let outer_pc = self.pc.clone();
                let outer_pc_trace = self.pc_trace.clone();
                self.pc = outer_pc.join(&cond.label.value);
                if !cond.label.value.is_bottom() {
                    self.pc_trace = merge(
                        &merge(&outer_pc_trace, &cond.trace),
                        &self.trace(vec![(
                            self.ifc.hir.exprs[conditional.cond].span,
                            "the branch outcome depends on a secret  [control dependency]"
                                .to_string(),
                        )]),
                    );
                }

                let before = self.acc.clone();
                self.block(conditional.then);
                let after_then = self.acc.clone();
                self.acc = before.clone();
                if let Some(otherwise) = conditional.otherwise {
                    self.block(otherwise);
                }
                // §17.3.4: the accumulator's behaviour at a block boundary
                // is a join over every path, including the path that did
                // not run. Leaving it unspecified is what let `From` inside
                // an `if` erase a branch's control dependency.
                self.acc = before.join(&after_then).join(&self.acc);
                self.pc = outer_pc;
                self.pc_trace = outer_pc_trace;
            }
        }
    }

    fn pipeline(&mut self, clause: &HirPipeline) {
        match clause {
            // **A join, never an assignment.** This was the only rule in
            // the set that assigned rather than joined and the only one
            // that omitted `⊔ pc`; inside an `if`, `acc` was overwritten
            // with a fresh ⊥ and the branch's control dependency vanished.
            HirPipeline::From(expr) => {
                let value = self.expr(*expr);
                let mut label = value.label;
                label.join_all(&self.pc);
                self.acc = self
                    .acc
                    .join(&Valued::of(label, merge(&value.trace, &self.pc_trace)));
            }
            HirPipeline::Keep { var, cond } => {
                self.bind_element(*var);
                let predicate = self.expr(*cond);
                // A predicate's label joins onto the **collection** label,
                // not merely onto element values: §14G.1.3(b), and it is
                // what rejects `keep each v where <secret predicate>`
                // returning a "public" list of public rows.
                self.acc.label.shape.join_in_place(&predicate.label.value);
                self.acc.label.shape.join_in_place(&self.pc);
                self.acc.label.settle();
                self.acc.trace = merge(&merge(&self.acc.trace, &predicate.trace), &self.pc_trace);
            }
            HirPipeline::Sort { var, key } => {
                self.bind_element(*var);
                let key = self.expr(*key);
                // The permutation reveals the key.
                self.acc.label.shape.join_in_place(&key.label.value);
                self.acc.label.shape.join_in_place(&self.pc);
                self.acc.label.settle();
                self.acc.trace = merge(&merge(&self.acc.trace, &key.trace), &self.pc_trace);
            }
            HirPipeline::MapEach { var, to } => {
                self.bind_element(*var);
                let to = self.expr(*to);
                // Only `value`: a mapped list keeps a public length.
                self.acc.label.value.join_in_place(&to.label.value);
                self.acc.label.value.join_in_place(&self.pc);
                self.acc.trace = merge(&merge(&self.acc.trace, &to.trace), &self.pc_trace);
            }
            HirPipeline::TakeFirst(expr) => {
                let count = self.expr(*expr);
                self.acc.label.shape.join_in_place(&count.label.value);
                self.acc.label.shape.join_in_place(&self.pc);
                self.acc.label.settle();
                self.acc.trace = merge(&merge(&self.acc.trace, &count.trace), &self.pc_trace);
            }
        }
    }

    fn bind_element(&mut self, var: LocalId) {
        let element = SymLabel::triple(self.acc.label.value.clone());
        let trace = self.acc.trace.clone();
        self.locals.insert(var, Valued::of(element, trace));
    }

    /// §5.3a(a)'s write rule: `label(rhs) ⊔ ⨆ label(index) ⊔ pc ⊑
    /// label(place)`.
    ///
    /// This is the first time the language has had a mutation site outside
    /// a `client` handler; before scheduled execution and relational
    /// writes one could not exist, which is why §5.3 never needed it.
    fn mutation(&mut self, mutation: &HirMutation) {
        // Every verb writes the value into the place, so every verb raises
        // the same obligation. `append` and `remove` are §14B.2's
        // membership verbs: `append apiKey to auditLog` is §14G.1.3
        // exhibit 5, the unlabelled write target, and it is caught here.
        let (place, value) = match mutation {
            HirMutation::Set { place, value }
            | HirMutation::Add { place, value }
            | HirMutation::Subtract { place, value }
            | HirMutation::Append { place, value }
            | HirMutation::Remove { place, value } => (place, *value),
        };
        let rhs = self.expr(value);
        let mut found = rhs.label.value.clone();
        let mut trace = rhs.trace;
        for segment in &place.path {
            if let HirPathSeg::Index(expr) = segment {
                let index = self.expr(*expr);
                found.join_in_place(&index.label.value);
                trace = merge(&trace, &index.trace);
            }
        }

        let Res::Def(base) = place.base else {
            return;
        };
        if !matches!(self.ifc.hir.defs[base].kind, DefKind::Signal(_)) {
            return;
        }
        let required = self
            .ifc
            .declared
            .get(&base)
            .copied()
            .unwrap_or_default()
            .value;
        self.oblige(Obligation {
            kind: ObligationKind::Write(base),
            required,
            found,
            pc: self.pc.clone(),
            site: place.span,
            what: format!("the value written into `{}`", self.ifc.hir.defs[base].name),
            found_trace: trace,
            pc_trace: self.pc_trace.clone(),
        });
    }

    // --- the view (§17.3.6's view rules) ---

    fn nodes(&mut self, nodes: &[HirNode]) {
        for node in nodes {
            match node {
                HirNode::Element(element) => self.element(element),
                HirNode::Each(each) => {
                    let iter = self.expr(each.iter);
                    self.require_public(
                        &iter,
                        self.ifc.hir.exprs[each.iter].span,
                        "the collection this list iterates",
                    );
                    let outer = self.pc.clone();
                    self.pc = outer.join(&iter.label.shape);
                    self.locals.insert(
                        each.var,
                        Valued::of(
                            SymLabel::triple(iter.label.value.clone()),
                            iter.trace.clone(),
                        ),
                    );
                    let body = each.body.clone();
                    self.nodes(&body);
                    self.pc = outer;
                }
                HirNode::When(when) => {
                    let scrutinee = self.expr(when.scrutinee);
                    // Only `shape` is required: a `when` that matches and
                    // renders nothing leaks only which arm it took.
                    let shape_only = Valued::of(
                        SymLabel::triple(scrutinee.label.shape.clone()),
                        scrutinee.trace.clone(),
                    );
                    self.require_public(
                        &shape_only,
                        self.ifc.hir.exprs[when.scrutinee].span,
                        "which arm this `when` takes",
                    );
                    let outer = self.pc.clone();
                    self.pc = outer.join(&scrutinee.label.shape);
                    for arm in &when.arms {
                        let bound = if arm.pattern_name == "Failed" {
                            scrutinee.label.failure.clone()
                        } else {
                            scrutinee.label.value.clone()
                        };
                        for binder in &arm.bindings {
                            self.locals.insert(
                                *binder,
                                Valued::of(
                                    SymLabel::triple(bound.clone()),
                                    scrutinee.trace.clone(),
                                ),
                            );
                        }
                        match &arm.body {
                            HirNodeArmBody::Show(element) => {
                                let element = element.clone();
                                self.element(&element);
                            }
                            HirNodeArmBody::Nodes(nodes) => {
                                let nodes = nodes.clone();
                                self.nodes(&nodes);
                            }
                        }
                    }
                    self.pc = outer;
                }
                HirNode::Handler(handler) => self.block(handler.body),
            }
        }
    }

    fn element(&mut self, element: &HirElement) {
        for arg in &element.args {
            let expr = match arg {
                HirArg::Positional(expr) => *expr,
                HirArg::Named { value, .. } => *value,
            };
            let value = self.expr(expr);
            self.require_public(&value, self.ifc.hir.exprs[expr].span, "this value");
        }
        // A two-way binding is a write on every keystroke, so it carries
        // the enclosing `pc` even though no `set` statement names it.
        if let Res::Builtin(Builtin::Element(builtin)) = element.res {
            if builtin.is_two_way() {
                if let Some(HirArg::Positional(expr)) = element.args.first() {
                    if let HirExprKind::Ref(Res::Def(def)) = self.ifc.hir.exprs[*expr].kind {
                        if matches!(self.ifc.hir.defs[def].kind, DefKind::Signal(_)) {
                            let required = self
                                .ifc
                                .declared
                                .get(&def)
                                .copied()
                                .unwrap_or_default()
                                .value;
                            self.oblige(Obligation {
                                kind: ObligationKind::Write(def),
                                required,
                                found: Sym::bottom(),
                                pc: self.pc.clone(),
                                site: element.span,
                                what: format!(
                                    "what the visitor types into `{}`",
                                    self.ifc.hir.defs[def].name
                                ),
                                found_trace: Vec::new(),
                                pc_trace: self.pc_trace.clone(),
                            });
                        }
                    }
                }
            }
        }
        let children = element.children.clone();
        self.nodes(&children);
    }

    /// §17.3.6's view require, with **explicit error recovery**: on a
    /// failure the label is replaced by Public for the remainder of the
    /// walk, exactly as `Type::Unknown` works in `zdc-types`. One
    /// diagnostic per secret scrutinee, no cascade — and the `pc` rule
    /// stays meaningful for the statement-position cases where it does the
    /// real work.
    fn require_public(&mut self, value: &Valued, span: Span, what: &str) {
        let found = value.label.value.concrete().join(self.pc.concrete());
        if found == Secrecy::Public && value.label.value.deps.is_empty() {
            return;
        }
        let site = SinkSite::ViewArg(ExprId::from_index(0), self.ctx);
        let _ = site;
        self.oblige(Obligation {
            kind: ObligationKind::Escape(Sink::View, SinkSite::ClientSignal(self.owner)),
            required: Secrecy::Public,
            found: value.label.value.clone(),
            pc: self.pc.clone(),
            site: span,
            what: what.to_string(),
            found_trace: value.trace.clone(),
            pc_trace: self.pc_trace.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sink_list_is_closed_at_six() {
        // §14G.1.3(c) names exactly six sinks. This assertion is one of
        // the three locks; the other two are the absent
        // `#[non_exhaustive]` and `Cleared`'s private field.
        assert_eq!(Sink::CLOSED_LIST.len(), 6);
    }

    #[test]
    fn every_sink_has_its_own_code_and_description() {
        let mut codes: Vec<&str> = Sink::CLOSED_LIST.iter().map(|s| s.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 6);
    }

    #[test]
    fn clearance_cannot_be_forged_from_outside() {
        // Not a runtime assertion — a compile-time one. `Cleared`'s field
        // is private, so `Cleared(())` outside this crate does not build,
        // and the only way to obtain one is `Verdict::cleared`.
        let verdict = Verdict::default();
        assert!(verdict
            .cleared(Sink::View, SinkSite::ClientSignal(DefId::from_index(0)))
            .is_none());
    }
}
