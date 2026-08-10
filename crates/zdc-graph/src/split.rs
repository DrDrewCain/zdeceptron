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
use zdc_hir::{ArenaId as _, DefId, DefKind, ExprId, Hir, PlaceId};
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
    /// Sink 3: a `static` signal is written to a file in the bundle
    /// (§14C.3b's `emitting`).
    ///
    /// This carried "Unconstructible: the grammar has no build-output
    /// construct (§17.7)" until `emitting` was added with the `static`
    /// placement, at which point the grammar acquired exactly that
    /// construct and nothing started emitting the edge. Sink 3 stayed
    /// declared, listed in `Sink::CLOSED_LIST`, and checked nowhere.
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
/// What identifies one write, for a map that must not conflate two.
///
/// Both arms are allocated fresh by instantiation, which is the property
/// a `Span` lacks: a component's body is copied per call site and keeps
/// its spans (#13).
///
/// Two arms rather than one id, because the two kinds of write are
/// genuinely different things. A `set` statement has a place; a two-way
/// `Input` binding has no statement at all — §18.1's `Site::Bind` exists
/// precisely because there is nothing to point at — so it is identified
/// by the expression naming the signal it binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WriteId {
    /// The left-hand side of a mutation.
    Place(PlaceId),
    /// The signal reference inside a two-way binding.
    Bound(ExprId),
}

#[derive(Debug, Clone, Default)]
pub struct TierSplit {
    pub roots: Vec<Root>,
    pub members: BTreeMap<RootId, BTreeMap<DefId, MemberForm>>,
    pub reached_by: BTreeMap<(DefId, RootId), (DefId, Span)>,
    pub crossings: BTreeMap<(ExprId, Ctx), Crossing>,
    pub mutations: BTreeMap<(MutSite, Ctx), MutCrossing>,
    /// The same crossings, keyed by the place written: its span, the
    /// context, and the signal it names.
    ///
    /// §17.2.5 fatal 2 gives a mutation site an *ordinal* identity so a
    /// two-way binding — which has no place to point at — is addressable
    /// at all. Code generation has the place in hand and not the ordinal,
    /// and recounting the ordinals in a second traversal is exactly the
    /// kind of duplicated walk that drifts. One map, filled once.
    ///
    /// Keyed on the write's own identity, which instantiation allocates
    /// fresh per copy.
    ///
    /// It used to be keyed on `(Span, Ctx, DefId)`. Instantiation copies a
    /// component's body once per call site and keeps the spans, so
    /// `set votes to n` written once inside `VoteCard` is two mutations
    /// with one span — and if the two instances are passed
    /// differently-placed signals, one is a `Command` and the other is
    /// `Local`. The signal in the key made that collision rarer and not
    /// impossible: two instances writing the *same* top-level signal share
    /// the `DefId` too, and then whichever the fixpoint reached last
    /// decided both (#13).
    ///
    /// `PlaceId` is the fix rather than a wider composite key, because it
    /// is an identity rather than a set of properties that happen to
    /// differ. This was the last load-bearing `Span` in a map key; the
    /// span-keyed map in `ifc.rs` is deliberate and documented there, and
    /// de-duplicates diagnostics rather than claiming identity.
    pub mutations_at: BTreeMap<(WriteId, Ctx), MutCrossing>,
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
    /// What a mutation of `signal` at this place becomes in this context.
    pub fn mutation_at(&self, place: PlaceId, ctx: Ctx) -> Option<&MutCrossing> {
        self.mutations_at.get(&(WriteId::Place(place), ctx))
    }

    /// Whether a mutation of `signal` at this place is, in **any** context
    /// it is reached from, a command a browser sends.
    ///
    /// Context-insensitive on purpose. One `set` inside a shared function
    /// may be a local write from a server root and a command from a client
    /// one; the integrity direction has to answer for the worst of them,
    /// because the endpoint the command reaches accepts what any browser
    /// posts to it whatever else calls the same function.
    /// Keyed on the place rather than on its span, for the reason
    /// [`TierSplit::mutations_at`] is (#13). The signal is no longer part
    /// of the question: one place writes one signal, so asking for both
    /// was asking the same thing twice.
    pub fn is_commanded(&self, place: PlaceId) -> bool {
        self.mutations_at.iter().any(|((write, _), crossing)| {
            *write == WriteId::Place(place) && matches!(crossing, MutCrossing::Command { .. })
        })
    }

    /// The endpoint a root generates, if it generates one.
    pub fn endpoint_of(&self, root: RootId) -> Option<&Endpoint> {
        self.endpoints.iter().find(|e| e.root == root)
    }

    /// The crossing recorded at a read site.
    pub fn crossing_at(&self, expr: ExprId, ctx: Ctx) -> Option<&Crossing> {
        self.crossings.get(&(expr, ctx))
    }

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
                // The other three cross no boundary the *type* can see:
                // an inlined `static` value is in the bundle, a store read
                // is performed by the root that reads it, and a lifted
                // cell arrives as a parameter. Written out rather than
                // wildcarded — a new crossing defaulting to `Direct` is a
                // `Remote of T` that never appears (§5.2).
                Some(Crossing::Direct | Crossing::Inline)
                | Some(Crossing::Store { .. })
                | Some(Crossing::Lift { .. }) => return ReadKind::Direct,
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
        // The browser's own store answers synchronously, so a read of one
        // crosses nothing. This is `Direct` for the same reason a `client`
        // read is, and `zdc_types::read_kind` puts it in the same row.
        (R::Client, _, P::Remembered) => Crossing::Direct,
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
        // A `remembered` value reaches a server derivation the same way a
        // `client` one does — the browser sends it as an argument — and
        // by the same route it does not reach a trigger, which has no
        // browser to ask. Nothing on a server has ever seen this store.
        (R::Server, K::View, P::Remembered) => Crossing::Lift {
            target: DefId::from_index(0),
        },
        (R::Server, _, P::Remembered) => Crossing::Rejected { code: "E0302" },
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

/// E0317's crossing message, written once because five sites raise it.
///
/// `subject` is the phrase naming what the program wrote, so the sentence
/// reads as one claim rather than as a template with a slot in it.
fn handle_error(subject: &str, span: Span) -> GraphError {
    GraphError::new(
        "E0317",
        // Inside the 200-character inline budget with the longest subject
        // any of the five sites writes, which is what keeps the whole
        // sentence on one line beside the caret. The rest of the argument
        // — why there is no wire form, and why `starting` is different —
        // is `zdc explain E0317`.
        format!(
            "{subject}, and a `{}` is a live object in one runtime's memory with no form on the \
             wire. Only a `foreign`'s line or `client` state with `starting` can hold one.",
            zdc_ast::HANDLE_TYPE_NAME
        ),
        span,
    )
    .with_label("nothing can send this anywhere")
}

/// E0317's **lifetime** message: the handle would go somewhere it could be
/// replaced, and nothing would release the one it replaced.
///
/// A separate sentence from [`handle_error`] because it is a separate
/// fact. That one is about the wire — a handle has no encoding, so it
/// cannot cross. This one is about *time*: `client` state can hold a
/// handle, and what it may not do is hold a second one, because the
/// language has no `destroy` to run on the first.
fn handle_lifetime_error(subject: &str, repair: &str, span: Span) -> GraphError {
    GraphError::new(
        "E0317",
        format!(
            "{subject}, so it would replace the handle it holds — and nothing releases the one \
             replaced. {repair}"
        ),
        span,
    )
    .with_label("nothing releases what this drops")
}

/// Why a build-time output path cannot be used, or `None` if it can.
///
/// The check is on the *written* path rather than on the resolved one: a
/// path is refused at compile time, so no build ever gets the chance to
/// write outside the directory it was told to write into.
pub fn unusable_path(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("is empty");
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Some("is an absolute path");
    }
    if path.contains(':') {
        return Some("names a drive or a scheme");
    }
    if path
        .split(['/', '\\'])
        .any(|segment| segment == ".." || segment == ".")
    {
        return Some("climbs out of the bundle");
    }
    if path.ends_with('/') || path.ends_with('\\') {
        return Some("names a directory rather than a file");
    }
    None
}

/// §17.2.7's `classify_write`.
pub fn classify_write(ctx: Ctx, target: SignalPlacement) -> MutCrossing {
    use Region as R;
    use RootKind as K;
    use SignalPlacement as P;

    match (ctx.region, ctx.kind, target) {
        (_, _, P::Static) => MutCrossing::Rejected { code: "E0310" },

        (R::Client, _, P::Client) => MutCrossing::Local,
        // A write lands in the browser's own store, which the browser
        // reaches without asking anybody. No command, no endpoint.
        (R::Client, _, P::Remembered) => MutCrossing::Local,
        (R::Client, _, P::Server) => MutCrossing::Rejected { code: "E0311" },
        (R::Client, _, P::Durable | P::DurablePerVisitor) => MutCrossing::Command { root: CLIENT },

        (R::Server, _, P::Client) => MutCrossing::Rejected { code: "E0312" },
        // E0312 for the reason a `client` write is: a server invocation
        // does not reach into a browser, and reaching into its *disk* is
        // further still.
        (R::Server, _, P::Remembered) => MutCrossing::Rejected { code: "E0312" },
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
        self.handle_declaration_checks();
        self.handle_write_checks();
        for (id, def) in self.hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            let placement = placement_of(signal.placement);

            // E0313. §5.3: only `server` and `durable` signals may be
            // secret, because the other two live where the reader is.
            if signal.secret && !placement.may_be_secret() {
                self.out.diagnostics.push(GraphError::new(
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
                ));
            }

            // §14C.3b's sub-requirement, and its three preconditions.
            if let Some(emitted) = &signal.emits {
                self.emission_checks(&def.name, placement, &signal.ty, emitted);
                // Sink 3, recorded rather than ruled on: the file lands in
                // the bundle, so whoever fetches the site can read it. The
                // split does not decide whether that is legal — IFC does,
                // exactly as it does for sink 6.
                self.out.boundary.push(BoundaryEdge::BuildOutput {
                    def: id,
                    path: emitted.path.clone(),
                });
            }

            // E0321. §5.5: a store holds what was put in it, so a
            // placement whose value is *kept* cannot also be a placement
            // whose value is *recomputed*. `remembered` is a store on the
            // browser's side of the network and inherits the rule whole:
            // a derived signal is recomputed from its inputs on every
            // load, which would overwrite the entry that survived the
            // reload with something that did not.
            if matches!(
                placement,
                SignalPlacement::Durable | SignalPlacement::DurablePerVisitor
            ) && !signal.is_source
            {
                self.out.diagnostics.push(GraphError::new(
                    "E0321",
                    format!(
                        "`{}` is `durable` and derived. Durable is storage, not computation.",
                        def.name
                    ),
                    def.span,
                ));
            }
            if placement == SignalPlacement::Remembered && !signal.is_source {
                self.out.diagnostics.push(
                    GraphError::new(
                        "E0321",
                        format!(
                            "`{}` is `remembered` and derived. A `remembered` value is one the \
                             browser kept, and a derived signal is one recomputed from its \
                             inputs — so this would overwrite what survived the reload every \
                             time the page loaded.",
                            def.name
                        ),
                        def.span,
                    )
                    .with_help(
                        "Write `starting` for the value on a browser that has never run this \
                         program. If the point is to compute it, declare it `client` and let \
                         the value it reads be the `remembered` one.",
                    ),
                );
            }
        }
    }

    /// E0317 — where a `Handle` may be written, which is three places.
    ///
    /// A handle refers to an object in **one** JavaScript heap. It is not a
    /// value with a wire form that this compiler declines to emit; there is
    /// no wire form to emit, because what would be encoded is an identity
    /// inside a process. `runtime/wire.js` therefore has no tag for one and
    /// cannot grow one.
    ///
    /// So the rule is not a policy but a transcription of that fact: a
    /// handle may be written as a `foreign`'s parameter type, as its result
    /// type, or as the type of a `client` signal declared `starting` — bare
    /// in all three, and nowhere else. Everything else this pass can see is
    /// somewhere a value crosses, persists, or is silently replaced —
    ///
    /// * a `server`, `durable` or `static` signal: read across a boundary
    ///   by definition, where `classify` would make it `Crossing::Remote`,
    ///   `Crossing::Store` or `Crossing::Inline` and every one of those
    ///   serialises.
    /// * a `record` field: a record is what crosses an endpoint, and
    ///   `Remote of List of Row` puts every field on the wire.
    /// * a `release`'s `gives`: a release exists to move a value across the
    ///   secrecy boundary, and an opaque one cannot be looked at to decide
    ///   whether that is safe.
    /// * anything under `List of`, `Option of`, `Map of`, `Pair of` — and
    ///   `Remote of`, which is the case §17 names outright. Nesting is
    ///   refused rather than reasoned about: a container of handles has no
    ///   encoding either, and admitting one would mean writing a
    ///   marshalling rule for a value that has none.
    ///
    /// # The `client` `starting` signal, and why it is safe
    ///
    /// #276 refused `state` outright and gave the reason: a derived signal
    /// recomputes, there is no `destroy` to run on the value it replaces,
    /// and a handle signal would therefore drop a live WebGL context on
    /// every recomputation — the leak `examples/tree/draw.js` kept a
    /// `mount`/`update` split to avoid. That reason is exactly right and it
    /// is exactly narrower than the rule it was used to justify. It is an
    /// argument about **replacement**, not about storage.
    ///
    /// So the rule this draws is the argument's own shape: a handle may
    /// live in a signal that is *never replaced*. Three conditions make
    /// that true and each is checked separately —
    ///
    /// 1. **`client`.** A handle is an object in the browser's heap, so any
    ///    other placement is the crossing case above.
    /// 2. **`starting`, not `from`.** A source signal's initialiser is
    ///    evaluated once, when the bundle is loaded. A derived one
    ///    recomputes, which is the leak.
    /// 3. **Never written.** `set`, `add`, `append` and the rest would put
    ///    a second handle where the first was, which is the same leak
    ///    written by hand. [`Splitter::handle_write_checks`] refuses every
    ///    one, at the write.
    ///
    /// Together those give a handle exactly one lifetime, and it is a
    /// lifetime the language can state: **the document's**. It is acquired
    /// once, it is never replaced, and it is released when the page is —
    /// which is when a browser reclaims a WebGL context anyway. Nothing is
    /// promised beyond that. A program that wants to release one *sooner*
    /// calls its disposer as an ordinary effect (`do disposeOf with r is
    /// gl`), and that is a call the program makes rather than an obligation
    /// this compiler enforces.
    ///
    /// What was rejected, and why:
    ///
    /// * **A `destroy` obligation on the type**, so that a replaced handle
    ///   were released automatically. It needs the compiler to know which
    ///   method disposes of which host object, which is a second
    ///   declaration form and a runtime protocol — and it would still be
    ///   guessing, because `renderer.dispose()` does not release the canvas
    ///   and `scene.clear()` does not release the geometry.
    /// * **A scoped form** — acquire, use, release at the end of a block.
    ///   Wrong shape for the problem: a renderer is acquired for the life
    ///   of the page, not for the life of a statement, and a construct that
    ///   released it at the end of a block would be unusable for the one
    ///   case this exists to serve.
    /// * **Allowing `from` and accepting the leak.** A handle that dropped
    ///   a WebGL context on every recomputation is worse than no feature at
    ///   all: the failure is silent, it is cumulative, and it ends with a
    ///   blank canvas after the browser stops granting contexts.
    ///
    /// Checked here rather than in the type checker because the split runs
    /// first and everything after it reads the crossings this pass records
    /// (§17.1.3). A handle that reached `classify` would already be a
    /// boundary the checker had been asked to describe.
    fn handle_declaration_checks(&mut self) {
        for (_, def) in self.hir.defs.iter() {
            match &def.kind {
                DefKind::Signal(signal) => self.handle_signal_checks(def, signal),
                DefKind::Record(record) => {
                    for field in &record.fields {
                        self.reject_handle(
                            &field.ty,
                            field.span,
                            &format!("`{}` is a field of `{}`", field.name, def.name),
                        );
                    }
                }
                DefKind::Release(release) => self.reject_handle(
                    &release.gives,
                    def.span,
                    &format!("`{}` is what a release gives", def.name),
                ),
                DefKind::Choice(choice) => {
                    for variant in &choice.variants {
                        for field in &variant.fields {
                            self.reject_handle(
                                &field.ty,
                                field.span,
                                &format!("`{}` is a field of `{}`", field.name, variant.name),
                            );
                        }
                    }
                }
                // The two positions a handle is admitted in, plus the
                // declarations that write no types at all. A `foreign`'s
                // `takes` and `gives` lines are the boundary itself rather
                // than a value crossing one, so `Handle` is legal there —
                // but only bare, which is what `reject_nested_handle`
                // checks.
                DefKind::Foreign(foreign) => {
                    for (ty, local) in foreign.param_types.iter().zip(foreign.params.iter()) {
                        let local = &self.hir.locals[*local];
                        self.reject_nested_handle(
                            ty,
                            local.span,
                            &format!("`{}` is a parameter of `{}`", local.name, def.name),
                        );
                    }
                    match &foreign.result {
                        zdc_ast::ForeignResult::Value(ty) | zdc_ast::ForeignResult::New(ty) => self
                            .reject_nested_handle(
                                ty,
                                def.span,
                                &format!("`{}` gives it", def.name),
                            ),
                        // Neither writes a result type, so neither can
                        // write a handle into one.
                        zdc_ast::ForeignResult::View | zdc_ast::ForeignResult::Nothing => {}
                    }
                }
                DefKind::Function(_) | DefKind::View(_) | DefKind::Component(_) => {}
            }
        }
    }

    /// The third condition: **nothing writes a handle signal.**
    ///
    /// Acquiring once is only half of "never replaced" — a `set` in a
    /// handler puts a second host object where the first was, and the first
    /// is gone with nothing having released it. That is the same leak a
    /// derived signal would have, written by hand, so it gets the same
    /// refusal.
    ///
    /// Walked over `sites_of`, which is context-free and is the same list
    /// the fixpoint below classifies, so this sees every write the split
    /// sees and each of them exactly once. Raised in `declaration_checks`
    /// rather than inside `Splitter::write`, because that runs once per
    /// (site, context) and would report one mistake several times.
    ///
    /// **A handle signal is still readable and still reactive.** Nothing
    /// here touches reads: `do addTo with parent is scene, child is mesh`
    /// reads two handle signals and is exactly what this feature is for.
    /// What is refused is putting a different object in the box.
    fn handle_write_checks(&mut self) {
        let ids: Vec<DefId> = self.hir.defs.iter().map(|(id, _)| id).collect();
        for id in ids {
            for site in sites_of(self.hir, id) {
                let (Site::Write { signal, span, .. } | Site::Bind { signal, span, .. }) = site
                else {
                    continue;
                };
                let DefKind::Signal(target) = &self.hir.defs[signal].kind else {
                    continue;
                };
                if !target.ty.mentions_handle() {
                    continue;
                }
                let name = self.hir.defs[signal].name.clone();
                self.out.diagnostics.push(handle_lifetime_error(
                    &format!("this line writes to `{name}`"),
                    "Acquire it once with `starting`.",
                    span,
                ));
            }
        }
    }

    /// The one position a handle may be *stored* in, and the three things
    /// that have to hold for it (see [`Splitter::handle_declaration_checks`]).
    ///
    /// Each is refused separately and names the one that failed, because
    /// they are three different mistakes with three different repairs.
    fn handle_signal_checks(&mut self, def: &zdc_hir::Def, signal: &zdc_hir::Signal) {
        if !signal.ty.mentions_handle() {
            return;
        }
        // A container of handles has no encoding either, so nesting is
        // refused before anything else is considered.
        if !signal.ty.is_bare_handle() {
            self.out
                .diagnostics
                .push(handle_error(&format!("`{}` is state", def.name), def.span));
            return;
        }
        let placement = placement_of(signal.placement);
        if placement != SignalPlacement::Client {
            self.out.diagnostics.push(handle_error(
                &format!("`{}` is `{}` state", def.name, placement.describe()),
                def.span,
            ));
            return;
        }
        if !signal.is_source {
            self.out.diagnostics.push(handle_lifetime_error(
                &format!("`{}` is derived and recomputes", def.name),
                "Write `starting`, which is evaluated once.",
                def.span,
            ));
        }
    }

    /// Refuse a `Handle` written anywhere in `ty`.
    fn reject_handle(&mut self, ty: &zdc_ast::TypeExpr, span: Span, subject: &str) {
        if ty.mentions_handle() {
            self.out.diagnostics.push(handle_error(subject, span));
        }
    }

    /// Refuse a `Handle` written anywhere in `ty` **except** bare.
    fn reject_nested_handle(&mut self, ty: &zdc_ast::TypeExpr, span: Span, subject: &str) {
        if ty.mentions_handle() && !ty.is_bare_handle() {
            self.out.diagnostics.push(handle_error(subject, span));
        }
    }

    /// §14C.3b: a `static` signal may be written to a file at build time.
    ///
    /// Three things have to hold, and each is refused separately so the
    /// diagnostic names the one that failed. The placement, because only a
    /// `static` signal has a value at build time to write. The type,
    /// because a file's contents are text. The path, because a generated
    /// file belongs in the bundle and an absolute or climbing path is not
    /// in the bundle.
    fn emission_checks(
        &mut self,
        name: &str,
        placement: SignalPlacement,
        ty: &zdc_ast::TypeExpr,
        emitted: &zdc_ast::Emitted,
    ) {
        if placement != SignalPlacement::Static {
            self.out.diagnostics.push(
                GraphError::new(
                    "E0314",
                    format!(
                        "`{name}` is `{}` and `emitting`, but a generated file is written once, \
                         at build time, from a value that exists then. `{}` state has no value \
                         at build time.",
                        placement.describe(),
                        placement.describe()
                    ),
                    emitted.span,
                )
                // E0314's own caret label says "this is a value, not a
                // place", which is true of the write site the code was
                // written for and false of this one: here the caret is on
                // an `emitting` clause. The site knows, so the site says.
                .with_label("this clause runs at build time")
                .with_help(
                    "Declare it `static`, which is the placement whose value is computed by the \
                     build (spec §14C.3b).",
                ),
            );
            return;
        }

        let is_text = matches!(ty, zdc_ast::TypeExpr::Named(named) if named.text == "Text");
        if !is_text {
            self.out.diagnostics.push(
                GraphError::new(
                    "E0315",
                    format!(
                        "`{name}` is written to `{}`, so it is the contents of a file and has to \
                         be `Text`.",
                        emitted.path
                    ),
                    emitted.span,
                )
                .with_help(
                    "Derive the file's text from this state in another `static` signal, and emit \
                     that one (spec §14C.3b).",
                ),
            );
        }

        if let Some(reason) = unusable_path(&emitted.path) {
            self.out.diagnostics.push(
                GraphError::new(
                    "E0316",
                    format!("`{name}` is written to `{}`, which {reason}.", emitted.path),
                    emitted.span,
                )
                .with_help(
                    "A generated file goes in the bundle, so its path is relative to the bundle \
                     root — `rss.xml`, or `feeds/posts.xml`.",
                ),
            );
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
                // A `client` or `server` signal is recomputed wherever it
                // is a member, so its initialiser is its body in every
                // root it reaches. A `remembered` signal's `starting`
                // expression is the value it takes on a browser that has
                // never seen this program, so it is emitted and evaluated
                // in the browser exactly as a `client` one is — the store
                // read is what may override it, not what replaces the
                // body.
                SignalPlacement::Client | SignalPlacement::Server | SignalPlacement::Remembered => {
                    true
                }
            },
            // A view and a function are bodies by definition. A `record`,
            // a `choice` and a `component` never reach here at all —
            // `form_of` says why — and answering `true` for them is what
            // this arm used to say by accident; it is harmless because
            // the caller then finds no initialiser to walk.
            DefKind::View(_)
            | DefKind::Function(_)
            // A release has a body like a function's, and it is emitted as
            // one, so it is walked in every root it is a member of.
            | DefKind::Release(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_)
            | DefKind::Foreign(_) => true,
        }
    }

    fn form_of(&self, def: DefId, root: RootId) -> MemberForm {
        match &self.hir.defs[def].kind {
            DefKind::View(_) => MemberForm::View,
            // A release is emitted as an ordinary server-side function. The
            // rules that make it a *release* are checked, not emitted:
            // nothing about the generated code differs, which is why §19.1
            // can say a call site does not advertise the crossing.
            DefKind::Function(_) | DefKind::Release(_) => MemberForm::Function,
            DefKind::Signal(signal) => match placement_of(signal.placement) {
                SignalPlacement::Static => MemberForm::Inlined,
                SignalPlacement::Durable | SignalPlacement::DurablePerVisitor if root != BUILD => {
                    MemberForm::StoreRead
                }
                // The BUILD root evaluates a durable initialiser for the
                // manifest, so there it is an ordinary binding — which is
                // the case the guard above lets fall through to here.
                SignalPlacement::Durable | SignalPlacement::DurablePerVisitor => {
                    MemberForm::Binding
                }
                // A `remembered` cell is an ordinary client binding whose
                // initialiser consults the browser's store first. It is
                // not a `StoreRead`: that form means an endpoint answers,
                // and here nothing leaves the tab.
                SignalPlacement::Client | SignalPlacement::Server | SignalPlacement::Remembered => {
                    MemberForm::Binding
                }
            },
            // A `record` or `choice` declares a type and emits nothing.
            // Nothing reaches one — `sites_of` records no edge to a type
            // declaration, and `orphan_pass` seeds only signals and
            // functions — so it is never a member of any root. A
            // `component` is unreachable for the same two reasons, plus a
            // third: instantiation already wrote its body out at every call
            // site, so the declaration that is left names nothing.
            // A `foreign` is unreachable for the same reasons: it emits
            // inline at each call site, so `sites_of` records no call edge
            // to one and it names no symbol a root could hold.
            DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_)
            | DefKind::Foreign(_) => {
                unreachable!(
                    "a type, component, or foreign declaration is never a member of a root"
                )
            }
        }
    }

    fn site(&mut self, def: DefId, root: RootId, ctx: Ctx, site: Site) {
        match site {
            // A `foreign` emits inline and has no body to reach, so it
            // contributes no edge to the member graph. It is recorded as
            // its own site kind for REL-PURE, which asks a different
            // question of the same call.
            Site::ForeignCall { .. } => {}
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
                place,
                signal,
                site,
                op,
                path,
                span,
            } => self.write(
                def,
                root,
                ctx,
                signal,
                site,
                op,
                path,
                span,
                WriteId::Place(place),
            ),
            Site::Bind {
                place,
                signal,
                site,
                span,
            } => self.write(
                def,
                root,
                ctx,
                signal,
                site,
                MutOp::Set,
                Vec::new(),
                span,
                WriteId::Bound(place),
            ),
            Site::NotAPlace { name, span } => {
                self.out.diagnostics.push(GraphError::new(
                    "E0314",
                    format!(
                            "`{name}` is not somewhere a value can be put. `add`, `subtract` and \
                             `set` write into `state`, and `{name}` is a value rather than a place."
                        ),
                    span,
                ));
            }
            Site::Media { span } => {
                if ctx.region != Region::Client {
                    self.out.diagnostics.push(
                        GraphError::new(
                            "E0362",
                            format!(
                                "`media` asks the browser whether it matches a query, and this \
                                 code runs in {}, where there is no browser to ask.",
                                ctx.describe()
                            ),
                            span,
                        )
                        .with_notes(self.out.path_from_root(def, root, self.hir))
                        .with_help(
                            "A media query is answered by the display the visitor is looking \
                             at. Read it into a `client` signal, and send that to the server \
                             if the server needs to know.",
                        ),
                    );
                }
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
                        .with_notes(self.out.path_from_root(def, root, self.hir)),
                    );
                }
            }
            // A build capability is answered by the compiler while the
            // compiler is running. There is no later moment at which one
            // could be answered at all, so this is not a permission check
            // that could be relaxed — outside build-time evaluation there
            // is nobody to ask.
            Site::Build { capability, span } => {
                if ctx.region != Region::Static {
                    self.out.diagnostics.push(
                        GraphError::new(
                            "E0361",
                            format!(
                                "`build {}` {}, and it is only readable while the build is \
                                 running. This code runs in {}.",
                                capability.name(),
                                capability.describe(),
                                ctx.describe()
                            ),
                            span,
                        )
                        .with_notes(self.out.path_from_root(def, root, self.hir))
                        .with_help(
                            "Read it into a `static` signal and read that signal here instead. \
                             A `static` value is computed once, at build time, and inlined \
                             (spec §14C.3b).",
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
                // The *key* does belong to the reading root, though. A
                // server region prints a signal read as a bare name, so a
                // root that reads one and does not hold it emits a name
                // nothing declares: `rank(votes)` against no `votes`, and a
                // `ReferenceError` on the first request. `walks_its_body`
                // keeps the initialiser out; `form_of` makes this a
                // `StoreRead`, which is the `await $store.get` line.
                self.work.push((signal, root));
                self.out
                    .reached_by
                    .entry((signal, root))
                    .or_insert((def, span));
                // A read is a reference, so it is a hoisting edge for the
                // same reason a `Direct` read is: the `const` this becomes
                // is written inside `handler`, and a module-scope reader of
                // it is out of scope.
                self.out
                    .direct_reads
                    .entry((def, root))
                    .or_default()
                    .insert(signal);
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
        write: WriteId,
    ) {
        let target = self.placement(signal);
        let recorded = match classify_write(ctx, target) {
            // The cell being written lives in *this* root, so the signal is
            // a member of it — exactly as a `Direct` read makes it one. A
            // signal that is only ever written would otherwise have no
            // declaration and no setter here, and the handler that writes
            // it would name a symbol that was never emitted. It joins
            // `direct_reads` for the same reason a read does: hoisting is a
            // question about references, and a write is one.
            MutCrossing::Local => {
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
                MutCrossing::Local
            }
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
        self.out.mutations.insert((site, ctx), recorded.clone());
        self.out.mutations_at.insert((write, ctx), recorded);
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
        // Each of these is the claim and nothing else: what was read, and
        // why this context cannot read it. The rule behind it — and the
        // repair — is `zdc explain <CODE>`.
        let message = match code {
            "E0301" => format!(
                "build-time state reads `{name}`, which is not build-time state. An initial \
                 value is computed once, at build time, with no browser and no store."
            ),
            "E0302" => format!(
                "a scheduled handler cannot read browser state, and `{name}` lives in browser \
                 memory. This handler runs on a schedule, with no browser."
            ),
            "E0303" => format!(
                "a trigger runs with no session, so there is no visitor whose partition it \
                 could read, and `{name}` is `durable per visitor`."
            ),
            // `code` is a `&'static str` rather than an enum, so Rust
            // demands a final arm here even though the domain is closed:
            // `read_crossing`, forty lines above, is the only thing that
            // builds a `Crossing::Rejected`, and it spells exactly the
            // three codes named above. Naming them and failing loudly is
            // the point — the arm this replaces silently rendered E0303's
            // prose for any code it had not been told about, which is a
            // wrong sentence rather than a missing one.
            other => unreachable!("`read_crossing` produced the unhandled rejection `{other}`"),
        };
        let mut notes = self.out.path_from_root(def, root, self.hir);
        notes.push((
            self.hir.defs[signal].span,
            format!("`{name}` is declared here"),
        ));
        let _ = ctx;
        self.out
            .diagnostics
            .push(GraphError::new(code, message, span).with_notes(notes));
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
        let message = match code {
            "E0310" => format!(
                "`{name}` is `static`, and `static` state is computed once at build time. \
                 There is nothing at run time to write into."
            ),
            "E0311" => format!(
                "the browser cannot write `{name}` directly: it is `server`-placed, and a \
                 `server` signal is recomputed from its inputs rather than assigned."
            ),
            "E0312" => format!(
                "code running in {} cannot write `{name}`, which lives in browser memory.",
                ctx.describe()
            ),
            // **`mut_crossing` can produce E0303 too**, and the wildcard
            // this replaces gave it E0312's sentence — "`{name}` lives in
            // browser memory" about a `durable per visitor` signal, which
            // is not merely unhelpful but false. Unreachable today only
            // because `per visitor` has no syntax (§14G.3a); the moment it
            // does, this is the arm that has to exist.
            "E0303" => format!(
                "a trigger runs with no session, so there is no visitor whose partition it \
                 could write, and `{name}` is `durable per visitor`."
            ),
            // Closed for the same reason `reject_read`'s is, and residual
            // for the same one: `code` is a `&'static str`, not an enum.
            other => unreachable!("`mut_crossing` produced the unhandled rejection `{other}`"),
        };
        let mut notes = self.out.path_from_root(def, root, self.hir);
        notes.push((
            self.hir.defs[signal].span,
            format!("`{name}` is declared here"),
        ));
        self.out
            .diagnostics
            .push(GraphError::new(code, message, span).with_notes(notes));
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
                DefKind::Function(_) => Ctx::CLIENT_VIEW,
                // The filter that built `unreached` admits only signals
                // and functions, so these never arrive. Named rather than
                // wildcarded so that a new `DefKind` has to be given an
                // orphan context on purpose. **A `release` is in this
                // group, and that is a gap rather than a decision**: the
                // filter does not admit one, so a release nothing calls
                // gets no orphan root and therefore none of the checking
                // §17.2.5 fatal 6 exists to preserve. It is inherited
                // unchanged from `feature/apps`, where the arm was a
                // wildcard that hid it.
                DefKind::View(_) | DefKind::Record(_) | DefKind::Choice(_) => Ctx::CLIENT_VIEW,
                DefKind::Component(_) | DefKind::Foreign(_) | DefKind::Release(_) => {
                    Ctx::CLIENT_VIEW
                }
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
    /// A member is emitted at module scope iff it names nothing `handler`
    /// owns; otherwise it is emitted inside `handler`, where the lifted
    /// parameters and the awaited store reads are lexically in scope.
    /// Least fixed point over call **and** read edges: lexical scope is a
    /// question about references, and a read is one.
    fn solve_hoisting(&mut self) {
        let mut needs: BTreeSet<(DefId, RootId)> = self
            .out
            .lifted
            .iter()
            .filter(|(_, lifted)| !lifted.is_empty())
            .map(|(key, _)| *key)
            .collect();

        // A lifted parameter is not the only thing `handler` owns. **Every
        // signal binding in a server root is written inside `handler`** —
        // a `Binding` because the root's members are emitted in dependency
        // order beside the parameters, a `StoreRead` because its `const` is
        // an `await` and there is nothing to await at module scope. So a
        // signal member is a seed in its own right, and a function that
        // names one is out of scope at module scope whether or not
        // anything in the root was lifted at all.
        //
        // Seeding from `lifted` alone missed both: `pick` naming a server
        // `pool` and `twice` naming a durable `total` were each emitted at
        // module scope above the `const` that binds the name, and each
        // threw `ReferenceError` on its first request.
        for (root, members) in &self.out.members {
            for (def, form) in members {
                if matches!(form, MemberForm::Binding | MemberForm::StoreRead) {
                    needs.insert((*def, *root));
                }
            }
        }

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
                // The other four roots are not endpoints. The client
                // bundle and the build host are singletons the platform
                // adapter mounts rather than calls; a trigger is invoked
                // by a schedule, not over the wire; and an orphan root is
                // checked and never emitted, so `root.emitted` has
                // already excluded it. Adding a fifth root origin that
                // *is* callable must be a compile error here, because an
                // endpoint the split forgets is an endpoint nothing
                // typechecks the wire signature of.
                RootOrigin::ClientBundle
                | RootOrigin::BuildHost
                | RootOrigin::Trigger(_)
                | RootOrigin::Orphan(_) => {}
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
                        // A call already on the frontier, and every site
                        // that is not a read or a call. A write, a
                        // two-way bind, a non-place and an `environment`
                        // read contribute no derivation edge: §17.5.2's
                        // graph is over initialisers, and none of the
                        // four appears in one. A new `Site` that did
                        // would have to be sorted here on purpose — which
                        // is why the guarded arm above is followed by
                        // `Site::Call` by name rather than by a wildcard
                        // that would also swallow the new one.
                        Site::Call { .. }
                        | Site::Media { .. }
                        | Site::Write { .. }
                        | Site::Bind { .. }
                        | Site::NotAPlace { .. }
                        // A `foreign` has no ZDeceptron body to descend
                        // into, and a build capability reads a file rather
                        // than a signal. Neither is a derivation edge
                        // between two signals, which is the only kind of
                        // edge this graph has.
                        | Site::ForeignCall { .. }
                        | Site::Build { .. }
                        | Site::Environment { .. } => {}
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
                .with_notes(notes),
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
                // `remembered` joins `client` here and not `server`: what
                // is missing is a cell, and no endpoint was ever going to
                // exist. The entry stays in the browser's store either
                // way — nothing removes it — which is worth knowing but is
                // not what this warning is about.
                SignalPlacement::Client | SignalPlacement::Static | SignalPlacement::Remembered => {
                    (
                        "W0331",
                        format!(
                            "`{}` is never read, so no cell and no setter are emitted for it.",
                            def.name
                        ),
                    )
                }
                // A `server` or `durable` signal is reached over the
                // wire, so what is missing is an endpoint rather than a
                // cell. Named rather than wildcarded: a fifth placement
                // would otherwise inherit whichever sentence happened to
                // be last, and the two sentences describe different
                // artefacts.
                SignalPlacement::Server
                | SignalPlacement::Durable
                | SignalPlacement::DurablePerVisitor => (
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
            // Only a signal produces a `Read` site, so none of these is
            // ever asked. `client` is the answer that keeps a caller's
            // arithmetic on the safe side — it is the region that may
            // read nothing else — and it is written out per kind so that
            // a new `DefKind` that *can* be read has to say so here.
            DefKind::Function(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_)
            | DefKind::Foreign(_)
            | DefKind::Release(_) => placement_of(Placement::Client),
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
