//! Tier splitting — spec §17.2.
//!
//! **Not graph colouring.** There is no conflict relation and no chromatic
//! number. Placement is *declared*, on the left-hand side of a `state`
//! declaration, and it is *inherited*: a function runs wherever its inputs
//! are. So the pass is a least fixed point over `(DefId, RootId)` — the
//! product of the definition set and the root set — with one total
//! classifier applied at each read and each write, plus monomorphisation:
//! a definition reached from two regions is walked twice, in two contexts,
//! and may mean two different things in each.
//!
//! The pass consults **no inference result anywhere** (§17.6). That is
//! load-bearing, and it is why this runs before `zdc-types` rather than
//! after: the type of a cross-placement read depends on the crossing, so
//! types depend on placement and never the other way round.

use std::collections::{BTreeMap, BTreeSet};

use zdc_ast::Placement;
use zdc_hir::{ArenaId as _, DefId, DefKind, ExprId, Hir};
use zdc_lexer::Span;
use zdc_types::{ReadContext, ReadKind, SignalPlacement};

use crate::diag::GraphError;
use crate::root::{
    placement_of, region_of, CommandKey, Ctx, MutOp, MutSite, PathKeySeg, Region, Root, RootId,
    RootKind, RootOrigin, BUILD, CLIENT,
};
use crate::sites::{sites_of, Site};

/// What a read across a boundary becomes — §17.2.4's walk behaviour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Crossing {
    /// Same region. The target joins the *same* root; no artifact.
    Direct,
    /// A `static` value, evaluated on the build host and substituted here.
    Inline,
    /// A durable key read from the store in this root.
    Store { key: DefId, per_visitor: bool },
    /// **Stop.** The DCE cut and the `Remote of T` introduction, which are
    /// literally the same edge (§17.3.5).
    Remote { endpoint: RootId },
    /// **Stop descending.** The client owns the cell, so it sends it: the
    /// target becomes a parameter of this root.
    Lift { target: DefId },
    /// The read is not allowed at all.
    Rejected { code: &'static str },
}

/// What a write across a boundary becomes — §17.2.7's `classify_write`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutCrossing {
    Local,
    StoreWrite { key: DefId, per_visitor: bool },
    Command { root: RootId },
    Rejected { code: &'static str },
}

/// How a member of a root is emitted — §17.2.8.
///
/// "Print `members(r)`" hides four different emissions, and the difference
/// between them is exactly the difference between a working bundle and a
/// `TypeError: name is not a function`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberForm {
    /// A signal: `const x = <init>` inside the root.
    Binding,
    /// A function: a JavaScript function declaration.
    Function,
    /// A durable signal read here: `$store.get(k)`.
    StoreRead,
    /// A `static` signal: no symbol at all, the value is substituted.
    Inlined,
    /// The view. **Not in §17.2.8's list**, which has four forms and no
    /// way to emit the one definition §17.2.1 proves is a member of
    /// exactly one root. See the report.
    View,
}

/// A browser-visible fact the split records and the information-flow pass
/// rules on — §17.2.5 fatal 4.
///
/// The split does not decide whether these are legal; it emits two
/// structurally different edges and lets IFC decide, which is the correct
/// division of labour because the split runs before labels exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundaryEdge {
    /// Sink 6: the browser is sent the VALUE.
    LiveValue { key: DefId },
    /// Sink 6: the browser is told the key CHANGED.
    Invalidate { key: DefId, endpoint: RootId },
    /// Sinks 1 and 2.
    RemoteResult { endpoint: RootId, value: DefId },
    /// Sink 2.
    ViewRead { expr: ExprId },
    /// Sink 3. Unconstructible: the grammar has no build-output construct
    /// (§17.7).
    BuildOutput { def: DefId, path: String },
    /// Sink 5. Unconstructible: there is no trigger runtime (§17.7).
    TriggerFail { root: RootId },
}

/// What a generated endpoint is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointKind {
    /// A value the browser reads: `$remote(name, params)`.
    Value(DefId),
    /// A mutation the browser performs: `$call(name, args)`.
    Command(CommandKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub root: RootId,
    pub name: String,
    pub kind: EndpointKind,
    /// Ascending `DefId` — which is source declaration order, because
    /// `zdc-resolve` allocates every definition in one pass over
    /// `program.decls` before any body is looked at. First-read order was
    /// rejected: editing a function body would silently reorder the wire
    /// format (§17.2.9).
    pub params: Vec<DefId>,
}

/// Everything the split knows, and the only thing the passes after it read.
#[derive(Debug, Clone, Default)]
pub struct TierSplit {
    pub roots: Vec<Root>,
    pub members: BTreeMap<RootId, BTreeMap<DefId, MemberForm>>,
    pub reached_by: BTreeMap<(DefId, RootId), (DefId, Span)>,
    pub crossings: BTreeMap<(ExprId, Ctx), Crossing>,
    pub mutations: BTreeMap<(MutSite, Ctx), MutCrossing>,
    pub lifted: BTreeMap<(DefId, RootId), BTreeSet<DefId>>,
    pub params: BTreeMap<RootId, Vec<DefId>>,
    pub hoisted: BTreeMap<(DefId, RootId), bool>,
    pub reads_keys: BTreeMap<RootId, BTreeSet<DefId>>,
    pub writes_keys: BTreeMap<RootId, BTreeSet<DefId>>,
    pub depends: BTreeMap<RootId, BTreeSet<RootId>>,
    pub endpoints: Vec<Endpoint>,
    pub contexts: BTreeMap<DefId, BTreeSet<Ctx>>,
    pub exprs_of: BTreeMap<RootId, BTreeSet<ExprId>>,
    pub boundary: Vec<BoundaryEdge>,
    pub static_order: Vec<DefId>,
    pub diagnostics: Vec<GraphError>,
    /// Every call edge, per root, for the hoisting fixpoint.
    calls: BTreeMap<(DefId, RootId), BTreeSet<DefId>>,
    /// Every same-region **read** edge, per root. §17.2.8 says the
    /// hoisting fixpoint is "over `Call` edges only ... because it is a
    /// question about lexical scope". That is wrong, and `chain.zd`
    /// demonstrates it: `shout` reaches `greeting` by a `Direct` read
    /// rather than by a call, `greeting` is emitted *inside* `handler`
    /// because it closes over a lifted parameter, and a module-scope
    /// `shout` referring to it is a `ReferenceError`. Lexical scope is a
    /// question about **references**, and a read is one.
    direct_reads: BTreeMap<(DefId, RootId), BTreeSet<DefId>>,
}

impl TierSplit {
    pub fn root(&self, id: RootId) -> &Root {
        &self.roots[id.0 as usize]
    }

    pub fn errors(&self) -> impl Iterator<Item = &GraphError> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }

    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }

    /// Every root that is actually emitted. `Orphan` roots exist so that
    /// nothing escapes checking, and contribute to no artifact.
    pub fn emitted_roots(&self) -> impl Iterator<Item = (RootId, &Root)> {
        self.roots
            .iter()
            .enumerate()
            .filter(|(_, root)| root.emitted)
            .map(|(index, root)| (RootId(index as u32), root))
    }

    pub fn members_of(&self, root: RootId) -> impl Iterator<Item = (DefId, MemberForm)> + '_ {
        self.members
            .get(&root)
            .into_iter()
            .flat_map(|members| members.iter().map(|(id, form)| (*id, *form)))
    }

    /// Whether a definition is emitted into a root.
    pub fn is_member(&self, def: DefId, root: RootId) -> bool {
        self.members
            .get(&root)
            .is_some_and(|members| members.contains_key(&def))
    }

    /// Every definition emitted into the client bundle — §14A.1's
    /// provable dead-code elimination, as a set rather than as a hope.
    pub fn client_members(&self) -> BTreeSet<DefId> {
        self.members
            .get(&CLIENT)
            .map(|members| members.keys().copied().collect())
            .unwrap_or_default()
    }

    /// §17.2.10's "reached: hourly → ingest → name", as notes.
    pub fn path_from_root(&self, def: DefId, root: RootId, hir: &Hir) -> Vec<(Span, String)> {
        let mut chain: Vec<DefId> = vec![def];
        let mut at = def;
        let mut guard = 0;
        while let Some((from, _)) = self.reached_by.get(&(at, root)) {
            if *from == at || guard > 64 {
                break;
            }
            chain.push(*from);
            at = *from;
            guard += 1;
        }
        chain.reverse();
        chain
            .iter()
            .map(|id| {
                (
                    hir.defs[*id].span,
                    format!("reached through `{}`", hir.defs[*id].name),
                )
            })
            .collect()
    }
}

/// §17.1.4's interface, implemented for the real thing. `zdc-types` sees
/// only this trait, which is what keeps the dependency running one way.
impl zdc_types::Placements for TierSplit {
    fn read_contexts(&self, def: DefId) -> Vec<ReadContext> {
        let mut found: Vec<ReadContext> = self
            .contexts
            .get(&def)
            .map(|set| set.iter().map(|ctx| ctx.read_context()).collect())
            .unwrap_or_default();
        found.sort_by_key(|context| context.describe());
        found.dedup();
        // Never empty for a definition that exists: §17.2.6's orphan roots
        // guarantee it. A definition with no context at all would be
        // silently unchecked, which is §17.2.5 fatal 6.
        if found.is_empty() {
            found.push(ReadContext::Client);
        }
        found
    }

    fn read_kind_at(&self, expr: ExprId, context: ReadContext) -> ReadKind {
        for ctx in Ctx::ALL {
            if ctx.read_context() != context {
                continue;
            }
            match self.crossings.get(&(expr, ctx)) {
                Some(Crossing::Remote { .. }) => return ReadKind::Remote,
                Some(Crossing::Rejected { .. }) => {
                    return ReadKind::Forbidden("the placement pass rejected this read")
                }
                Some(_) => return ReadKind::Direct,
                None => {}
            }
        }
        // Not a signal read, or a read the walk never reached. Either way
        // the declared type is the answer.
        ReadKind::Direct
    }
}

/// §17.2.4 — §14G.1.4 as a total function.
///
/// Row 2 is derived and flagged as such: §14G.1.4's first row says "view /
/// `client` signal", and a `client`-placed `every` trigger is neither. It
/// is ruled `Remote`, because a client trigger has a browser attached.
pub fn classify(ctx: Ctx, target: SignalPlacement) -> Crossing {
    use Region as R;
    use RootKind as K;
    use SignalPlacement as P;

    match (ctx.region, ctx.kind, target) {
        (R::Client, _, P::Client) => Crossing::Direct,
        (R::Client, _, P::Static) => Crossing::Inline,
        (R::Client, _, P::Server | P::Durable | P::DurablePerVisitor) => Crossing::Remote {
            // Filled in by the caller, which owns root creation.
            endpoint: CLIENT,
        },

        (R::Static, _, P::Static) => Crossing::Direct,
        (R::Static, _, _) => Crossing::Rejected { code: "E0301" },

        (R::Server, _, P::Static) => Crossing::Inline,
        (R::Server, _, P::Server) => Crossing::Direct,
        (R::Server, K::View, P::Client) => Crossing::Lift {
            target: DefId::from_index(0),
        },
        (R::Server, _, P::Client) => Crossing::Rejected { code: "E0302" },
        (R::Server, _, P::Durable) => Crossing::Store {
            key: DefId::from_index(0),
            per_visitor: false,
        },
        (R::Server, K::View, P::DurablePerVisitor) => Crossing::Store {
            key: DefId::from_index(0),
            per_visitor: true,
        },
        (R::Server, _, P::DurablePerVisitor) => Crossing::Rejected { code: "E0303" },
    }
}

/// §17.2.7's `classify_write`.
pub fn classify_write(ctx: Ctx, target: SignalPlacement) -> MutCrossing {
    use Region as R;
    use RootKind as K;
    use SignalPlacement as P;

    match (ctx.region, ctx.kind, target) {
        (_, _, P::Static) => MutCrossing::Rejected { code: "E0310" },

        (R::Client, _, P::Client) => MutCrossing::Local,
        (R::Client, _, P::Server) => MutCrossing::Rejected { code: "E0311" },
        (R::Client, _, P::Durable | P::DurablePerVisitor) => MutCrossing::Command { root: CLIENT },

        (R::Server, _, P::Client) => MutCrossing::Rejected { code: "E0312" },
        (R::Server, _, P::Server) => MutCrossing::Local,
        (R::Server, _, P::Durable) => MutCrossing::StoreWrite {
            key: DefId::from_index(0),
            per_visitor: false,
        },
        (R::Server, K::View, P::DurablePerVisitor) => MutCrossing::StoreWrite {
            key: DefId::from_index(0),
            per_visitor: true,
        },
        (R::Server, _, P::DurablePerVisitor) => MutCrossing::Rejected { code: "E0303" },

        (R::Static, _, _) => MutCrossing::Rejected { code: "E0312" },
    }
}

/// Run the split. It never refuses to run: it is the first analysis, and
/// everything after it refuses instead (§17.1.3).
pub fn split(hir: &Hir) -> TierSplit {
    Splitter::new(hir).run()
}

struct Splitter<'a> {
    hir: &'a Hir,
    out: TierSplit,
    endpoint_roots: BTreeMap<DefId, RootId>,
    command_roots: BTreeMap<CommandKey, RootId>,
    seen: BTreeSet<(DefId, RootId)>,
    work: Vec<(DefId, RootId)>,
}

impl<'a> Splitter<'a> {
    fn new(hir: &'a Hir) -> Splitter<'a> {
        let mut out = TierSplit::default();
        // The two singletons always exist, possibly empty (§17.2.6).
        out.roots.push(Root {
            ctx: Ctx::CLIENT_VIEW,
            origin: RootOrigin::ClientBundle,
            span: Span::new(0, 0),
            emitted: true,
        });
        out.roots.push(Root {
            ctx: Ctx::STATIC_BUILD,
            origin: RootOrigin::BuildHost,
            span: Span::new(0, 0),
            emitted: true,
        });
        Splitter {
            hir,
            out,
            endpoint_roots: BTreeMap::new(),
            command_roots: BTreeMap::new(),
            seen: BTreeSet::new(),
            work: Vec::new(),
        }
    }

    fn run(mut self) -> TierSplit {
        self.declaration_checks();
        self.seed();
        self.fixpoint();
        self.orphan_pass();
        self.collect_params();
        self.solve_hoisting();
        self.collect_endpoints();
        self.derivation_cycle_check();
        self.unread_warnings();
        self.out
    }

    // --- declarations that need no walk at all ---

    fn declaration_checks(&mut self) {
        for (_, def) in self.hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            let placement = placement_of(signal.placement);

            // E0313. §5.3: only `server` and `durable` signals may be
            // secret, because the other two live where the reader is.
            if signal.secret
                && matches!(placement, SignalPlacement::Client | SignalPlacement::Static)
            {
                self.out.diagnostics.push(
                    GraphError::new(
                        "E0313",
                        format!(
                            "`{}` is declared `secret`, but it is `{}`-placed, and `{}` state is \
                             readable by whoever it lives with. A secret can only live where the \
                             reader is not.",
                            def.name,
                            placement.describe(),
                            placement.describe()
                        ),
                        def.span,
                    )
                    .with_help(
                        "Move it to `server` or `durable`, which the browser reaches only through \
                         a generated RPC (spec §5.3).",
                    ),
                );
            }

            // E0321. §5.5: durable is storage, not computation.
            if matches!(
                placement,
                SignalPlacement::Durable | SignalPlacement::DurablePerVisitor
            ) && !signal.is_source
            {
                self.out.diagnostics.push(
                    GraphError::new(
                        "E0321",
                        format!(
                            "`{}` is `durable` and derived. Durable is storage, not computation.",
                            def.name
                        ),
                        def.span,
                    )
                    .with_help(
                        "Write `starting` rather than `from`, and put the derivation in a `server` \
                         signal that reads this one (spec §5.5).",
                    ),
                );
            }
        }
    }

    // --- seeds (§17.2.6) ---

    fn seed(&mut self) {
        if let Some(view) = self.hir.view {
            self.work.push((view, CLIENT));
        }
        // A `durable` or `static` signal's initialiser is walked **only**
        // in the BUILD root, at `(Static, Build)` — §17.2.5 fatal 5. Its
        // value is written into `manifest.json` at build time, so it must
        // be computable with no browser, no request and no store.
        for (id, def) in self.hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            if matches!(
                placement_of(signal.placement),
                SignalPlacement::Durable
                    | SignalPlacement::DurablePerVisitor
                    | SignalPlacement::Static
            ) {
                self.work.push((id, BUILD));
            }
        }
    }

    // --- the fixpoint (§17.2.7) ---

    fn fixpoint(&mut self) {
        while let Some((def, root)) = self.work.pop() {
            if !self.seen.insert((def, root)) {
                continue;
            }
            let ctx = self.out.root(root).ctx;
            let form = self.form_of(def, root);
            self.out.members.entry(root).or_default().insert(def, form);
            self.out.contexts.entry(def).or_default().insert(ctx);

            if !self.walks_its_body(def, root) {
                continue;
            }

            for site in sites_of(self.hir, def) {
                self.site(def, root, ctx, site);
            }
        }
    }

    /// A durable or static signal contributes its initialiser to the BUILD
    /// root and to nothing else. Everywhere else it is a store read or an
    /// inlined constant, with no body of its own to walk.
    fn walks_its_body(&self, def: DefId, root: RootId) -> bool {
        match &self.hir.defs[def].kind {
            DefKind::Signal(signal) => match placement_of(signal.placement) {
                SignalPlacement::Durable
                | SignalPlacement::DurablePerVisitor
                | SignalPlacement::Static => root == BUILD,
                _ => true,
            },
            _ => true,
        }
    }

    fn form_of(&self, def: DefId, root: RootId) -> MemberForm {
        match &self.hir.defs[def].kind {
            DefKind::View(_) => MemberForm::View,
            DefKind::Function(_) => MemberForm::Function,
            DefKind::Signal(signal) => match placement_of(signal.placement) {
                SignalPlacement::Static => MemberForm::Inlined,
                SignalPlacement::Durable | SignalPlacement::DurablePerVisitor if root != BUILD => {
                    MemberForm::StoreRead
                }
                _ => MemberForm::Binding,
            },
        }
    }

    fn site(&mut self, def: DefId, root: RootId, ctx: Ctx, site: Site) {
        match site {
            Site::Call { callee, span } => {
                self.out
                    .reached_by
                    .entry((callee, root))
                    .or_insert((def, span));
                self.out
                    .calls
                    .entry((def, root))
                    .or_default()
                    .insert(callee);
                self.work.push((callee, root));
            }
            Site::Read { signal, expr, span } => self.read(def, root, ctx, signal, expr, span),
            Site::Write {
                signal,
                site,
                op,
                path,
                span,
            } => self.write(def, root, ctx, signal, site, op, path, span),
            Site::Bind { signal, site, span } => {
                self.write(def, root, ctx, signal, site, MutOp::Set, Vec::new(), span)
            }
            Site::NotAPlace { name, span } => {
                self.out.diagnostics.push(
                    GraphError::new(
                        "E0314",
                        format!(
                            "`{name}` is not somewhere a value can be put. `add`, `subtract` and \
                             `set` write into `state`, and `{name}` is a value rather than a place."
                        ),
                        span,
                    )
                    .with_help(
                        "Name a `state` declaration here. A parameter holds a copy of what was \
                         passed, so writing into it could not be observed by anyone (spec §17.2.7).",
                    ),
                );
            }
            Site::Environment { span } => {
                if ctx.region != Region::Server {
                    self.out.diagnostics.push(
                        GraphError::new(
                            "E0360",
                            format!(
                                "`environment` is only readable in `server` context, and this code \
                                 runs in {}.",
                                ctx.describe()
                            ),
                            span,
                        )
                        .with_notes(self.out.path_from_root(def, root, self.hir))
                        .with_help(
                            "Read it into a `server` signal and read that signal here instead \
                             (spec §5.6).",
                        ),
                    );
                }
            }
        }
    }

    fn read(
        &mut self,
        def: DefId,
        root: RootId,
        ctx: Ctx,
        signal: DefId,
        expr: ExprId,
        span: Span,
    ) {
        let target = self.placement(signal);
        let crossing = classify(ctx, target);
        self.out.exprs_of.entry(root).or_default().insert(expr);

        let recorded = match crossing {
            Crossing::Direct => {
                self.work.push((signal, root));
                self.out
                    .reached_by
                    .entry((signal, root))
                    .or_insert((def, span));
                self.out
                    .direct_reads
                    .entry((def, root))
                    .or_default()
                    .insert(signal);
                Crossing::Direct
            }
            Crossing::Inline => {
                self.work.push((signal, BUILD));
                Crossing::Inline
            }
            Crossing::Store { per_visitor, .. } => {
                self.out.reads_keys.entry(root).or_default().insert(signal);
                self.out.boundary.push(BoundaryEdge::Invalidate {
                    key: signal,
                    endpoint: root,
                });
                // §17.2.5 fatal 5: the initialiser goes to BUILD, never
                // into the reading root.
                self.work.push((signal, BUILD));
                Crossing::Store {
                    key: signal,
                    per_visitor,
                }
            }
            Crossing::Remote { .. } => {
                let endpoint = self.endpoint_root(signal);
                self.work.push((signal, endpoint));
                self.out
                    .reached_by
                    .entry((signal, endpoint))
                    .or_insert((signal, self.hir.defs[signal].span));
                self.out.depends.entry(root).or_default().insert(endpoint);
                self.out.boundary.push(BoundaryEdge::RemoteResult {
                    endpoint,
                    value: signal,
                });
                if matches!(
                    target,
                    SignalPlacement::Durable | SignalPlacement::DurablePerVisitor
                ) {
                    self.out
                        .reads_keys
                        .entry(endpoint)
                        .or_default()
                        .insert(signal);
                    self.out
                        .boundary
                        .push(BoundaryEdge::LiveValue { key: signal });
                }
                Crossing::Remote { endpoint }
            }
            Crossing::Lift { .. } => {
                self.out
                    .lifted
                    .entry((def, root))
                    .or_default()
                    .insert(signal);
                // The client must own the cell to be able to send it.
                self.work.push((signal, CLIENT));
                Crossing::Lift { target: signal }
            }
            Crossing::Rejected { code } => {
                self.reject_read(def, root, ctx, signal, code, span);
                Crossing::Rejected { code }
            }
        };

        self.out.crossings.insert((expr, ctx), recorded);
    }

    #[allow(clippy::too_many_arguments)]
    fn write(
        &mut self,
        def: DefId,
        root: RootId,
        ctx: Ctx,
        signal: DefId,
        site: MutSite,
        op: MutOp,
        path: Vec<PathKeySeg>,
        span: Span,
    ) {
        let target = self.placement(signal);
        let recorded = match classify_write(ctx, target) {
            MutCrossing::Local => MutCrossing::Local,
            MutCrossing::StoreWrite { per_visitor, .. } => {
                self.out.writes_keys.entry(root).or_default().insert(signal);
                MutCrossing::StoreWrite {
                    key: signal,
                    per_visitor,
                }
            }
            MutCrossing::Command { .. } => {
                let key = CommandKey {
                    signal,
                    op,
                    path: path.clone(),
                };
                let command = self.command_root(key, span);
                self.out
                    .writes_keys
                    .entry(command)
                    .or_default()
                    .insert(signal);
                self.out.depends.entry(root).or_default().insert(command);
                MutCrossing::Command { root: command }
            }
            MutCrossing::Rejected { code } => {
                self.reject_write(def, root, ctx, signal, code, span);
                MutCrossing::Rejected { code }
            }
        };
        self.out.mutations.insert((site, ctx), recorded);
    }

    fn reject_read(
        &mut self,
        def: DefId,
        root: RootId,
        ctx: Ctx,
        signal: DefId,
        code: &'static str,
        span: Span,
    ) {
        let name = self.hir.defs[signal].name.clone();
        let (message, help) = match code {
            "E0301" => (
                format!(
                    "build-time state reads `{name}`, which is not build-time state. A `durable` \
                     or `static` signal's initial value is written into `manifest.json` at build \
                     time, so it must be computable with no browser, no request and no store."
                ),
                "Give it a literal initial value, and write the computed one from a `server` \
                 signal (spec §17.2.5).",
            ),
            "E0302" => (
                format!(
                    "a scheduled handler cannot read browser state, and `{name}` lives in browser \
                     memory. This handler runs on a schedule, with no browser."
                ),
                "Read it from a view-rooted `server` signal instead, where the client supplies it \
                 as an RPC argument (spec §14G.1.4).",
            ),
            _ => (
                format!(
                    "a trigger runs with no session, so there is no visitor whose partition it \
                     could read, and `{name}` is `durable per visitor`."
                ),
                "Read a globally-scoped `durable` signal here (spec §14G.1.4).",
            ),
        };
        let mut notes = self.out.path_from_root(def, root, self.hir);
        notes.push((
            self.hir.defs[signal].span,
            format!("`{name}` is declared here"),
        ));
        let _ = ctx;
        self.out.diagnostics.push(
            GraphError::new(code, message, span)
                .with_notes(notes)
                .with_help(help),
        );
    }

    fn reject_write(
        &mut self,
        def: DefId,
        root: RootId,
        ctx: Ctx,
        signal: DefId,
        code: &'static str,
        span: Span,
    ) {
        let name = self.hir.defs[signal].name.clone();
        let (message, help) = match code {
            "E0310" => (
                format!(
                    "`{name}` is `static`, and `static` state is computed once at build time. \
                     There is nothing at run time to write into."
                ),
                "Declare it `client`, `server` or `durable` if it needs to change (spec §14C.3b).",
            ),
            "E0311" => (
                format!(
                    "the browser cannot write `{name}` directly: it is `server`-placed, and a \
                     `server` signal is recomputed from its inputs rather than assigned."
                ),
                "Write the `client` or `durable` state it is derived from, and let the compiler \
                 re-run the derivation (spec §5.5).",
            ),
            _ => (
                format!(
                    "code running in {} cannot write `{name}`, which lives in browser memory.",
                    ctx.describe()
                ),
                "Give the value back with `give` and let the browser write it (spec §5.2).",
            ),
        };
        let mut notes = self.out.path_from_root(def, root, self.hir);
        notes.push((
            self.hir.defs[signal].span,
            format!("`{name}` is declared here"),
        ));
        self.out.diagnostics.push(
            GraphError::new(code, message, span)
                .with_notes(notes)
                .with_help(help),
        );
    }

    // --- root creation, memoised (§17.5.1) ---

    fn endpoint_root(&mut self, signal: DefId) -> RootId {
        if let Some(root) = self.endpoint_roots.get(&signal) {
            return *root;
        }
        let id = RootId(self.out.roots.len() as u32);
        self.out.roots.push(Root {
            ctx: Ctx::SERVER_VIEW,
            origin: RootOrigin::Endpoint(signal),
            span: self.hir.defs[signal].span,
            emitted: true,
        });
        self.endpoint_roots.insert(signal, id);
        id
    }

    fn command_root(&mut self, key: CommandKey, span: Span) -> RootId {
        if let Some(root) = self.command_roots.get(&key) {
            return *root;
        }
        let id = RootId(self.out.roots.len() as u32);
        self.out.roots.push(Root {
            ctx: Ctx::SERVER_VIEW,
            origin: RootOrigin::Command(key.clone()),
            span,
            emitted: true,
        });
        self.command_roots.insert(key, id);
        id
    }

    // --- after convergence ---

    /// §17.2.5 fatal 6. A purely demand-driven root set silently deletes
    /// typechecking and every placement diagnostic for unreached code —
    /// verified: `dead.zd` produces two real type errors today and zero
    /// under a demand-driven root set. Orphan roots contribute to
    /// `contexts`, to `read_contexts` and to `diagnostics`, and to no
    /// emitted artifact.
    fn orphan_pass(&mut self) {
        let unreached: Vec<DefId> = self
            .hir
            .defs
            .iter()
            .filter(|(id, def)| {
                matches!(def.kind, DefKind::Signal(_) | DefKind::Function(_))
                    && !self.seen.iter().any(|(seen, _)| seen == id)
            })
            .map(|(id, _)| id)
            .collect();

        for id in unreached {
            if self.seen.iter().any(|(seen, _)| *seen == id) {
                continue;
            }
            let ctx = match &self.hir.defs[id].kind {
                DefKind::Signal(signal) => Ctx {
                    region: region_of(placement_of(signal.placement)),
                    kind: RootKind::View,
                },
                // A function nothing calls is checked as client code: it
                // is the least surprising reading, and it has no
                // cross-placement read to get wrong.
                _ => Ctx::CLIENT_VIEW,
            };
            let root = RootId(self.out.roots.len() as u32);
            self.out.roots.push(Root {
                ctx,
                origin: RootOrigin::Orphan(id),
                span: self.hir.defs[id].span,
                emitted: false,
            });
            self.work.push((id, root));
            self.fixpoint();
        }
    }

    /// §17.2.5 fatal 1. The endpoint's parameter set is the union over
    /// *members*, not the transitive-call closure from the root
    /// definition: a signal can enter a root by a `Direct` **read** edge,
    /// and the lift is then discovered under it rather than under a call.
    fn collect_params(&mut self) {
        for index in 0..self.out.roots.len() {
            let root = RootId(index as u32);
            let mut params: BTreeSet<DefId> = BTreeSet::new();
            if let Some(members) = self.out.members.get(&root) {
                for member in members.keys() {
                    if let Some(lifted) = self.out.lifted.get(&(*member, root)) {
                        params.extend(lifted.iter().copied());
                    }
                }
            }
            self.out
                .params
                .insert(root, params.into_iter().collect::<Vec<_>>());
        }
    }

    /// §17.2.8's hoisting, which replaces §16.3.12's byte-for-byte claim.
    ///
    /// A member is emitted at module scope iff it needs no lifted value;
    /// otherwise it is emitted inside `handler`, where the lifted
    /// parameters are lexically in scope. Least fixed point over `Call`
    /// edges only — this one *is* correctly call-only, because it is a
    /// question about lexical scope rather than about signatures.
    fn solve_hoisting(&mut self) {
        let mut needs: BTreeSet<(DefId, RootId)> = self
            .out
            .lifted
            .iter()
            .filter(|(_, lifted)| !lifted.is_empty())
            .map(|(key, _)| *key)
            .collect();

        let mut edges: BTreeMap<(DefId, RootId), BTreeSet<DefId>> = self.out.calls.clone();
        for (key, reads) in &self.out.direct_reads {
            edges.entry(*key).or_default().extend(reads.iter().copied());
        }

        loop {
            let mut changed = false;
            let edges: Vec<((DefId, RootId), BTreeSet<DefId>)> = edges
                .iter()
                .map(|(key, callees)| (*key, callees.clone()))
                .collect();
            for ((def, root), callees) in edges {
                if needs.contains(&(def, root)) {
                    continue;
                }
                if callees
                    .iter()
                    .any(|callee| needs.contains(&(*callee, root)))
                {
                    needs.insert((def, root));
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        for (root, members) in &self.out.members {
            for def in members.keys() {
                self.out
                    .hoisted
                    .insert((*def, *root), !needs.contains(&(*def, *root)));
            }
        }
    }

    fn collect_endpoints(&mut self) {
        let mut endpoints = Vec::new();
        for (index, root) in self.out.roots.iter().enumerate() {
            let id = RootId(index as u32);
            if !root.emitted {
                continue;
            }
            let params = self.out.params.get(&id).cloned().unwrap_or_default();
            match &root.origin {
                RootOrigin::Endpoint(def) => endpoints.push(Endpoint {
                    root: id,
                    name: self.hir.defs[*def].name.clone(),
                    kind: EndpointKind::Value(*def),
                    params,
                }),
                RootOrigin::Command(key) => endpoints.push(Endpoint {
                    root: id,
                    name: key.render(&self.hir.defs[key.signal].name),
                    kind: EndpointKind::Command(key.clone()),
                    params,
                }),
                _ => {}
            }
        }
        self.out.endpoints = endpoints;
    }

    /// §17.5.2. One node per signal; an edge `s → t` when `t` is read
    /// during evaluation of `s`'s initialiser, transitively through `Call`
    /// edges. **Every crossing kind contributes an edge**, so a client
    /// signal derived from a server signal derived from that client signal
    /// is reported as a cycle rather than discovered as an RPC storm.
    fn derivation_cycle_check(&mut self) {
        let mut edges: BTreeMap<DefId, BTreeSet<DefId>> = BTreeMap::new();
        for (id, def) in self.hir.defs.iter() {
            if !matches!(def.kind, DefKind::Signal(_)) {
                continue;
            }
            let mut reads: BTreeSet<DefId> = BTreeSet::new();
            let mut seen: BTreeSet<DefId> = BTreeSet::from([id]);
            let mut frontier = vec![id];
            while let Some(at) = frontier.pop() {
                for site in sites_of(self.hir, at) {
                    match site {
                        // Do not descend into the target's own initialiser:
                        // it contributes its own edges from its own root.
                        Site::Read { signal, .. } => {
                            reads.insert(signal);
                        }
                        Site::Call { callee, .. } if seen.insert(callee) => frontier.push(callee),
                        _ => {}
                    }
                }
            }
            edges.insert(id, reads);
        }

        for cycle in strongly_connected(&edges) {
            let names: Vec<String> = cycle
                .iter()
                .map(|id| self.hir.defs[*id].name.clone())
                .collect();
            let notes: Vec<(Span, String)> = cycle
                .iter()
                .map(|id| {
                    (
                        self.hir.defs[*id].span,
                        format!("`{}` is part of the cycle", self.hir.defs[*id].name),
                    )
                })
                .collect();
            self.out.diagnostics.push(
                GraphError::new(
                    "E0320",
                    format!(
                        "these signals are defined in terms of each other, so none of them has a \
                         value: {}.",
                        names.join(" → ")
                    ),
                    self.hir.defs[cycle[0]].span,
                )
                .with_notes(notes)
                .with_help(
                    "Break the cycle: one of them must start with a value rather than be derived \
                     from the others (spec §17.5.2).",
                ),
            );
        }

        self.out.static_order = topological(&edges);
    }

    /// §17.2.5 fatal 6's companion warnings. An unread `client` signal now
    /// costs no cell and no setter, which is a real change from §16.3.12's
    /// "every client signal is a seed" — so it is reported rather than
    /// silently dropped.
    fn unread_warnings(&mut self) {
        for (id, def) in self.hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            let reached_anywhere = self
                .out
                .contexts
                .get(&id)
                .is_some_and(|contexts| !contexts.is_empty());
            let emitted_anywhere = self
                .out
                .emitted_roots()
                .any(|(root, _)| self.out.is_member(id, root));
            if !reached_anywhere || emitted_anywhere {
                continue;
            }
            let (code, message) = match placement_of(signal.placement) {
                SignalPlacement::Client | SignalPlacement::Static => (
                    "W0331",
                    format!(
                        "`{}` is never read, so no cell and no setter are emitted for it.",
                        def.name
                    ),
                ),
                _ => (
                    "W0330",
                    format!(
                        "`{}` is never read, so no endpoint is generated for it.",
                        def.name
                    ),
                ),
            };
            self.out
                .diagnostics
                .push(GraphError::warning(code, message, def.span));
        }
    }

    fn placement(&self, signal: DefId) -> SignalPlacement {
        match &self.hir.defs[signal].kind {
            DefKind::Signal(s) => placement_of(s.placement),
            // Only a signal produces a `Read` site.
            _ => placement_of(Placement::Client),
        }
    }
}

/// Tarjan's strongly connected components, restricted to the ones that are
/// cycles: a component of size > 1, or a self-loop.
fn strongly_connected(edges: &BTreeMap<DefId, BTreeSet<DefId>>) -> Vec<Vec<DefId>> {
    let mut index: BTreeMap<DefId, usize> = BTreeMap::new();
    let mut low: BTreeMap<DefId, usize> = BTreeMap::new();
    let mut on_stack: BTreeSet<DefId> = BTreeSet::new();
    let mut stack: Vec<DefId> = Vec::new();
    let mut next = 0usize;
    let mut found: Vec<Vec<DefId>> = Vec::new();

    // Iterative, so a deep signal graph cannot overflow the Rust stack.
    for &start in edges.keys() {
        if index.contains_key(&start) {
            continue;
        }
        let mut call_stack: Vec<(DefId, usize)> = vec![(start, 0)];
        index.insert(start, next);
        low.insert(start, next);
        next += 1;
        stack.push(start);
        on_stack.insert(start);

        while let Some((at, position)) = call_stack.pop() {
            let neighbours: Vec<DefId> = edges
                .get(&at)
                .map(|set| set.iter().copied().collect())
                .unwrap_or_default();
            if position < neighbours.len() {
                call_stack.push((at, position + 1));
                let next_node = neighbours[position];
                if !edges.contains_key(&next_node) {
                    continue;
                }
                if let std::collections::btree_map::Entry::Vacant(slot) = index.entry(next_node) {
                    slot.insert(next);
                    low.insert(next_node, next);
                    next += 1;
                    stack.push(next_node);
                    on_stack.insert(next_node);
                    call_stack.push((next_node, 0));
                } else if on_stack.contains(&next_node) {
                    let candidate = index[&next_node];
                    let current = low[&at];
                    low.insert(at, current.min(candidate));
                }
                continue;
            }

            if let Some((parent, _)) = call_stack.last() {
                let child = low[&at];
                let current = low[parent];
                low.insert(*parent, current.min(child));
            }

            if low[&at] == index[&at] {
                let mut component = Vec::new();
                while let Some(popped) = stack.pop() {
                    on_stack.remove(&popped);
                    component.push(popped);
                    if popped == at {
                        break;
                    }
                }
                let self_loop = component.len() == 1
                    && edges
                        .get(&component[0])
                        .is_some_and(|set| set.contains(&component[0]));
                if component.len() > 1 || self_loop {
                    component.reverse();
                    found.push(component);
                }
            }
        }
    }
    found
}

/// Dependencies first. Only meaningful when the graph is acyclic, which
/// E0320 has just established.
fn topological(edges: &BTreeMap<DefId, BTreeSet<DefId>>) -> Vec<DefId> {
    let mut out: Vec<DefId> = Vec::new();
    let mut placed: BTreeSet<DefId> = BTreeSet::new();
    let mut guard = 0;
    while out.len() < edges.len() && guard <= edges.len() {
        for (id, deps) in edges {
            if placed.contains(id) {
                continue;
            }
            if deps
                .iter()
                .all(|dep| !edges.contains_key(dep) || placed.contains(dep))
            {
                placed.insert(*id);
                out.push(*id);
            }
        }
        guard += 1;
    }
    out
}
