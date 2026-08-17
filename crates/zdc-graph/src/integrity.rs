//! The integrity direction — spec §18.1, §19, and §21.7 as narrowed by §21.8.
//!
//! `secret` answers *who may learn this value*. `trusted` answers *who
//! chose it*. This module is the second question, and it is **default
//! closed**: a value is Untrusted unless it derives, by join alone, from
//! the grant set in [`Grant`]. There is no other way to become Trusted.
//!
//! # Why closed, and what that overturns
//!
//! §21.7 inverted the lattice, and the inversion is the one part of that
//! decision §21.8 leaves standing. A default-open lattice is sound only
//! if the enumeration of untrusted sources is complete, and three
//! consecutive designs shipped an enumeration that was not: §19.9 found a
//! channel through a selection predicate, §19.11 found one through a
//! `foreign`, and §21.8.4 found one through a two-way `Input` binding.
//! Under a closed set the question is not "did we list every way in?" but
//! "which of these eight grants applies?", which is answerable by a total
//! function — and [`Grant`] is written as one, deliberately, because
//! §21.8.4's own diagnosis is that *grant sets stated as prose tables need
//! to be stated as a total function before anything is built on them*.
//!
//! # What is **not** claimed
//!
//! **No robustness property.** Three independent adversarial passes broke
//! the soundness argument, the third (§21.8) after §21.7 had repaired the
//! second. These rules are built on §21.8.8 option 2's terms: keep the
//! declaration shape, the report, `limit`, REL-PLACE′, REL-CLOSED,
//! REL-PURE and REL-ARG **as review aids**, and withdraw the claim.
//!
//! Two of §21.8's breaks were live in the rules below. Both are repaired,
//! and each repair is marked at the point of use rather than hidden here:
//!
//! * **R1 — [`Grant::ForeignPure`] and [`rel_pure`] were stated over
//!   `is anywhere`.** §14E.2's own heading makes that word a claim about
//!   *which bundles a library may be linked into* — a linkability
//!   classification, not a purity one — so a query-string reader was
//!   honestly `is anywhere`, honestly Trusted, and attacker-chosen.
//!   §21.9 separates the two questions: placement stays on the `is` line
//!   and purity moves to `gives pure T`, a marker the author writes.
//!   The prelude's `clock` does not carry it, so its result is Untrusted
//!   and a release body that reaches it is refused.
//!
//!   **The marker is asserted, not checked.** It is a claim about
//!   arbitrary JavaScript, which is undecidable, and §14E.4's dev-mode
//!   check validates only the shape of a return value. What §21.9 buys is
//!   not verification: it is that the claim is now *declared at a
//!   conspicuous declaration* instead of *inferred from an answer to a
//!   different question*. An unchecked marker is an obligation moved onto
//!   a human, and §14E already accepts exactly that bargain for `takes`
//!   and `gives`.
//! * **R2 — G-SIG once granted Trusted to cells the program does not
//!   write.** That one *is* repaired here, in full, because the compiler
//!   already carries every mechanism the repair needs: [`Site::Bind`]
//!   records a two-way binding, the declaration records a `durable`
//!   placement, and `TierSplit::lifted` records a lifted `client` cell. So
//!   `examples/blog.zd`'s `query` and §19.9.1's `cards` both have a writer
//!   and are Untrusted. See [`Writers`], which also records why §21.7.3's
//!   verdict table is the side of that contradiction that is right.
//!
//! **Closing R1 does not make the design robust.** §21.8.8's residual
//! risks R3 (nothing bounds cumulative disclosure), R5 (both foreign
//! grants are asserted about third-party JavaScript and checked by
//! nobody) and R7's N2 (one visitor reading another's row is still a leak
//! that compiles) are untouched. The claim §21.7.10 made stays withdrawn.
//!
//! **R6 is narrowed, and only narrowed.** It reads *"a purity grant has no
//! argument chain for an attacker-reachability walk to follow"*, and its
//! consequence was that the grants §21.7 leans on were the ones no review
//! artifact reached. `zdc build --report` now reaches them: [`crate::report`]
//! enumerates every asserted grant with its declaration, its call sites and
//! the `release` bodies that depend on it. The walk itself is **not**
//! built, and giving the grant an argument chain would not build it — see
//! [`Grant::is_asserted`], which is the point of use where §21.8.3's
//! objection lives.
//!
//! # The claim, decided
//!
//! **DECIDED 2026-08-16, closing #212. It stays withdrawn, and it is now
//! withdrawn for a reason rather than pending one.** The four risks above
//! were listed against a lattice that has since been inverted and closed,
//! so each is re-argued here against the direction §21.7 settled. None of
//! the four is repaired by that direction, one of them is promoted, and
//! the reason the claim cannot be made is a check that is absent rather
//! than a check that is weak.
//!
//! **Robustness is a claim about declassification, and the rule for one
//! has two conjuncts. This pass has the first.** The property says an
//! attacker who supplies low-integrity inputs cannot influence what is
//! released, and the condition that enforces it asks two things at each
//! declassification: that the value released is high integrity, and that
//! *the decision to release it* is. [`crate::authority`]'s `rel_arg` is
//! the first, written out as an inference rule at its own doc comment and
//! quantified over the argument list. **The second is not written
//! anywhere.** The walk carries a program counter — `Walk::pc`, described
//! at its field as §18.1 semantics 11's — maintained across `if`, `when`
//! and `with`, and it is read at exactly two places: the binder a `with`
//! introduces, and `implicit_flow`, which is called from the A3 arm alone.
//! No release rule reads it. `a_browser_chosen_branch_chooses_which_release_runs`
//! is the program that shows what that costs — a text box picks which of
//! two releases runs, every argument is covered by a grant or an
//! endorsement, and the pass reports nothing at all.
//!
//! **And the missing conjunct cannot be discharged the textbook way
//! here.** The textbook repair is to refuse a declassification standing
//! under an untrusted `pc`, and it does not survive contact with a
//! `release`: a release is reached from a browser, so whoever is at the
//! browser decides when it is called and how often. The `pc` at a
//! client-reachable release call site is attacker-chosen by construction,
//! and a rule written that way refuses every program with a reason to
//! declare one. What stands in for it in that setting is a budget, which
//! is what `limit N per visitor` is for — and the budget is enforced by
//! nobody. `ReleaseBudget` reaches [`w_rel_01`], `zdc-doc`'s page,
//! `zdc-lsp`'s hover, and `zdc-types`, where §19.2 rule 5 makes a budgeted
//! release call at `Option of T` so that exhaustion cannot be forgotten.
//! It reaches no line of `zdc-codegen`, `zdc-host` or `zdc-store`, and
//! codegen's own call-site arm says why: *"a release is called exactly
//! like a function"*. So the type says a caller must handle running out,
//! and nothing emitted ever runs out.
//!
//! Building §11's own `judge` example with `limit 20 per visitor` is the
//! shortest way to see it: the handler is four lines and holds no counter,
//! and the client half is `$remote('result', [guess])` — a `remoteCell`
//! whose `effect` re-invokes the endpoint whenever an input signal
//! changes, which for a text box is once per keystroke. Twenty is the
//! number a reviewer reads at the declaration; the emitted program calls
//! it as often as somebody types. **R3 is therefore not a risk standing
//! beside the robustness question; in this language it is the robustness
//! question**, and it is open (#29).
//!
//! The other three, re-argued against the closed direction:
//!
//! * **R5 (#30).** A property that quantifies over attacks needs an
//!   attacker model, and this lattice's is *everything outside the eight
//!   grants*. Two of the eight are a human's word about JavaScript
//!   ([`Grant::is_asserted`]), so the domain the property would quantify
//!   over is chosen, per program, by the author it is meant to protect.
//!   `an_asserted_purity_marker_still_launders_and_that_is_r5_not_r1` is
//!   the measurement: one word on a `gives` line and §21.8.1's leak
//!   compiles again, clean. Closing the set made the enumeration
//!   *complete*; it was never going to make it *checked*. What it does
//!   buy is that the unchecked assumptions are **enumerable**, because
//!   [`Grant::CLOSED_LIST`] is a constant and [`Grant::is_asserted`] is
//!   total over it — which is a report this compiler could honestly emit,
//!   and is not a robustness claim.
//! * **R6 (#31).** `a_pure_foreign_of_no_arguments_leaves_nothing_to_walk`
//!   is the shape, and it is the prelude's own `clock` with one word
//!   added: `gives pure Whole` with no `takes` is Trusted by `⨆ ∅`, passes
//!   REL-PURE, and offers a reachability walk no argument to follow. So
//!   the aid that would let a reviewer decide whether R5's assertion is
//!   load-bearing stops exactly where the assertion is. The fold is not
//!   the defect and cannot be repaired: `⨆ ∅ = Trusted` is correct, as
//!   [`Authority::join_all`] says at more length.
//! * **R7's N2 (#32).** Robustness states what an attacker can cause an
//!   *observer* to learn, and the observer here is undifferentiated. Both
//!   lattices range over placements rather than principals, so nothing can
//!   say whose durable row a value is. The claim cannot be *stated* at the
//!   granularity the risk lives at, which is prior to its being
//!   unenforceable. Integrity answers who chose a value and
//!   confidentiality answers who may learn it; neither substitutes for the
//!   other, and the closed direction is entirely on the first side.
//!
//! **A claim made today would also be vacuous, which is the strongest
//! reason not to make one.** [`crate::ifc`] treats a `release` as an
//! ordinary function — *"what makes it a release is checked elsewhere, not
//! emitted"* — so no secret reaches a public sink through one, and the
//! language reference says so under what is not implemented. A robustness
//! claim now would be true of an empty set of declassifications and would
//! be read as a promise about the construct that will declassify when one
//! does.
//!
//! **What may be claimed instead, exactly.** For the six grants
//! [`Grant::is_asserted`] answers `false` for, a Trusted label records
//! that the value's provenance is one of a fixed list the compiler
//! checked, and the list is complete because [`Integrity::flow`] is a
//! total match over `HirExprKind` with no wildcard and [`Grant`] is not
//! `#[non_exhaustive]`. That is a claim about this analysis. It is not a
//! claim about a user's program, and E-REL-08's own explanation already
//! declines to make one: *"a call with no E-REL-08 is not thereby a call
//! nobody steered."*
//!
//! **What would change the answer.** Three things, all three, in this
//! order:
//!
//! 1. **A budget that is enforced and composes** — held where it survives
//!    a session, keyed on a principal a cleared cookie does not re-mint,
//!    and summed across declarations rather than per declaration (#29,
//!    which is asking #32's question about partitions from the other
//!    side). Until one exists the second conjunct has nothing to stand on,
//!    and `Option of T` is a type with a dead variant.
//! 2. **The `pc` conjunct written down as a rule** — `rel_arg` reading
//!    `Walk::pc`, `Walk::nodes` maintaining it across a view's `if` and
//!    `when` as `Walk::stmt` already does inside a body, and a ruling on
//!    what a release call site the browser cannot reach means, since that
//!    is the only kind the rule could ever accept.
//! 3. **The asserted grants enumerated in a report** (#30), so the
//!    attacker model a program assumes is something a reviewer reads
//!    rather than something they supply.
//!
//! What all three would buy is still not the literature's robust
//! declassification. It is *"an attacker cannot cause more than N
//! evaluations of the declassifiers this build enumerates, given the
//! assertions this build lists"* — bounded, conditional, and worth writing
//! down on the day it is true. `zdc-diagnostics`' `no_robustness_claim`
//! scans the shipped text for the unbounded version, and the day that
//! sentence is written is the day that scan has to be re-argued rather
//! than amended.
//!
//! **And what this costs, said rather than discovered later.**
//! Authentication is the feature that was waiting on this answer, because
//! authenticating is declassifying a decision derived from credentials a
//! visitor supplied — the one place robustness is load-bearing rather than
//! decorative. The answer here is that the property it wanted is not
//! available, so an authentication design cannot be built on it and has to
//! carry its own argument. §13 already lists authentication among the v1
//! non-goals; this is why that is the right place for it and not a
//! scheduling accident.
//!
//! Callers must not turn any of this into a promise. `limit` is not a
//! cumulative disclosure bound (§21.8.7), and nothing here establishes
//! that a program is free of laundering.

use std::collections::{BTreeMap, BTreeSet};

use zdc_hir::{
    BuildCapability, Builtin, DefId, DefKind, ExprId, Hir, HirArg, HirExprKind, LocalId, Res,
};
use zdc_lexer::Span;

use crate::authority::{match_args, Flow, Solution};
use crate::diag::GraphError;
use crate::sites::{sites_of, Site};
use crate::split::TierSplit;

/// The two points of the integrity lattice, ordered `Trusted ⊑ Untrusted`.
///
/// `Default` is [`Authority::Untrusted`], and that is the whole design:
/// under §21.7.0 a value is Untrusted unless a grant says otherwise, so
/// the type's own default is the safe one and a missing case in a walk
/// fails closed rather than open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Authority {
    Trusted,
    #[default]
    Untrusted,
}

impl Authority {
    /// `⊔`. The only operation the pass performs, which is what makes it
    /// polymorphic in its lattice (§18.1).
    pub fn join(self, other: Authority) -> Authority {
        match (self, other) {
            (Authority::Trusted, Authority::Trusted) => Authority::Trusted,
            (Authority::Untrusted, _) | (_, Authority::Untrusted) => Authority::Untrusted,
        }
    }

    /// The join of a sequence. `⨆ ∅ = Trusted`, the lattice's bottom.
    ///
    /// This identity was the visible edge of R1: a no-argument
    /// `is anywhere` foreign joined the empty set and came out Trusted,
    /// and the prelude's `clock` is exactly that shape. The fold was never
    /// the defect — a genuinely pure function of no arguments *is* a
    /// constant, so Trusted is the right answer for one. The defect was
    /// admitting `clock` to the fold at all, and after §21.9 only a
    /// `gives pure T` declaration reaches it.
    pub fn join_all(labels: impl IntoIterator<Item = Authority>) -> Authority {
        labels.into_iter().fold(Authority::Trusted, Authority::join)
    }

    pub fn is_trusted(self) -> bool {
        self == Authority::Trusted
    }
}

/// The closed set of ways to become Trusted — §21.7.3, as narrowed by §21.8.
///
/// Deliberately **not** `#[non_exhaustive]`, for the same reason
/// [`crate::ifc::Sink`] is not: adding a grant must break every downstream
/// `match`, so a ninth way into the Trusted half cannot be added without
/// every consumer being made to rule on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Grant {
    /// **G-LIT** — a literal. Checked by the grammar.
    Literal,
    /// **G-ENV** — `environment "K"`. The operator set it (§17.3.4).
    Environment,
    /// **G-VIS** — `visitor`, in any `(Server, View)` expression (§20.2 rule 6).
    ///
    /// **In the set, but unawardable today.** §20's visitor principal is
    /// not built, so no expression form resolves to it and
    /// [`Integrity::of`] can never return this. It is listed rather than
    /// omitted because the closed set is the whole argument: a grant that
    /// exists in the design and not in this enum would be a grant nobody
    /// had to rule on when §20 lands.
    Visitor,
    /// **G-FGN-T** — a `foreign` declaring `gives trusted T`.
    ///
    /// Unconditionally Trusted whatever its arguments were. Asserted by a
    /// human and checked by nobody, at build or ever (§21.7.5 assumption
    /// 2, residual risk R5).
    ForeignTrusted,
    /// **G-FGN-P** — a `foreign` declaring `gives pure T`; result is the
    /// join of its arguments.
    ///
    /// **This is R1's repair, and the rename is part of it.** The grant
    /// was `G-FGN-A`, awarded for `is anywhere` — an answer to *which
    /// bundles may this be linked into*. The join-of-arguments rule is
    /// correct for a pure function and unsound for anything else, so it is
    /// now conditional on a marker that answers the question it needs
    /// answered. A `foreign` without the marker is Untrusted whatever its
    /// arguments were, and whatever its placement is.
    ///
    /// Asserted by a human and checked by nobody, exactly as
    /// [`Grant::ForeignTrusted`] is (R5).
    ForeignPure,
    /// **G-SIG** — a read of a signal declared `trusted`, or of one with
    /// no write site anywhere whose initialiser is Trusted.
    ///
    /// The second clause is where §21.8.4's R2 bit. See [`Writers`].
    Signal,
    /// **G-BLD** — a build-time read at a literal path inside the project
    /// tree; the file is in the operator's version control.
    ///
    /// **Awardable since `build read` and `build list` landed.** The
    /// doc comment here used to say *"in the set, but unawardable today,
    /// because the `static` placement's build-time read is not built"*,
    /// and that premise stopped being true the moment §4.4's capabilities
    /// arrived. Note what §21.7.3 settled — `static` gets **no blanket
    /// grant**; §18.1 semantics 9's `static` half is overturned, so a
    /// build that fetches a feed through an ungranted `foreign` yields
    /// Untrusted state, and only the literal-path case is a grant.
    ///
    /// *Literal* is the whole of it. `build read "content/about.md"` names
    /// a file the author committed, which is the same bargain
    /// [`Grant::Environment`] makes about a variable the operator set.
    /// `build read path` for a `path` that came from anywhere else names
    /// whatever chose the path, so it is Untrusted, and that is what stops
    /// a capability laundering an untrusted path into trusted content.
    Build,
    /// **G-REL** — a `release` parameter named by a `trusted` clause.
    ///
    /// Site-local and result-transparent: it discharges REL-ARG at that
    /// release's call sites and does nothing anywhere else (§19.10.3(a)).
    Release,
}

impl Grant {
    /// Every grant there is. §19.5's audit trail is complete only if this
    /// list is, so it is a constant a test can count rather than a set a
    /// reader must assemble from prose.
    pub const CLOSED_LIST: [Grant; 8] = [
        Grant::Literal,
        Grant::Environment,
        Grant::Visitor,
        Grant::ForeignTrusted,
        Grant::ForeignPure,
        Grant::Signal,
        Grant::Build,
        Grant::Release,
    ];

    /// The spec's own name for this grant, for the report and for tests
    /// that should not assert on prose.
    pub fn code(self) -> &'static str {
        match self {
            Grant::Literal => "G-LIT",
            Grant::Environment => "G-ENV",
            Grant::Visitor => "G-VIS",
            Grant::ForeignTrusted => "G-FGN-T",
            Grant::ForeignPure => "G-FGN-P",
            Grant::Signal => "G-SIG",
            Grant::Build => "G-BLD",
            Grant::Release => "G-REL",
        }
    }

    /// Whether the grant is asserted by a human rather than checked by the
    /// compiler.
    ///
    /// §19.5 needs this to mark the entries a reviewer must actually read,
    /// and [`crate::report`] prints it beside every one of them — which is
    /// the half of residual risk R6 that is now closed. Before that, the
    /// two grants this returns `true` for were the two nothing enumerated.
    ///
    /// **It is still not `attacker_reachable`, and that field is still not
    /// emitted.** §21.8.3's objection was never that nobody had written the
    /// walk. It is that a purity grant has no argument for one to follow,
    /// and — the part worth stating, because it is what makes the gap
    /// permanent rather than pending — *giving it one would not help*. A
    /// `gives pure` foreign's channel is inside the JavaScript: §21.8.1's
    /// `queryParam` takes a string literal and reads `location.search`, so
    /// a walk over its arguments terminates at a literal and answers "no
    /// attacker-controlled value reaches this grant" about the grant a
    /// visitor steers with a query string. An available, cheap, false
    /// answer is worse than none, and §21.8.7 withdrew the field for
    /// exactly that. So the report says which assertions exist and which
    /// releases rest on them, and claims nothing about who can reach one.
    pub fn is_asserted(self) -> bool {
        match self {
            Grant::ForeignTrusted | Grant::ForeignPure => true,
            Grant::Literal
            | Grant::Environment
            | Grant::Visitor
            | Grant::Signal
            | Grant::Build
            | Grant::Release => false,
        }
    }
}

/// Which signals have a writer, and therefore which fail G-SIG's second
/// clause.
///
/// **This is the repair for §21.8.4 (residual risk R2), in full.** G-SIG as
/// written asks whether a signal *"has no write site anywhere in the
/// program"*, and §21.7.5 item 6 decides that by *"a whole-program
/// reachability query over **statement forms**"*. That query answers a
/// question about the **program text**, and it was being read as a question
/// about **who can put a value in the cell**. Three kinds of writer are not
/// statement forms:
///
/// * **A two-way `Input` binding.** The browser writes the signal on every
///   keystroke and there is no `set` for the query to find, so
///   `examples/blog.zd`'s `query` — a text box — came out Trusted.
///   [`Site::Bind`] records one, so the repair is to ask the site walk
///   rather than the statement forms.
/// * **A `durable` cell.** The store outlives the build. *"No write site in
///   this program"* does not entail *"holds its initialiser"*, because a
///   previous deployment, a migration or a database client is not in this
///   program's statement forms. This is §21.8.4's `Crossing::Store`
///   conjunct, decided at the declaration — where it is **exact**, since a
///   durable cell is externally writable however it is read.
/// * **A lifted `client` cell.** §21.8.4's `Crossing::Lift` conjunct. The
///   browser owns the cell and *sends* the value, so what arrives at a
///   server region is whatever the browser chose to send, bound or not.
///   Decided over [`TierSplit::lifted`] rather than over the placement, so
///   that a client signal nothing lifts keeps the grant.
///
/// # Why this is a repair and not a fourth patch
///
/// §21.7.3's own verdict table asserts that §19.9.1's `cards` is Untrusted;
/// the rule as written makes it Trusted, because `launder.zd` contains no
/// `set cards`. The table is right and the rule is wrong, and §21.8.4 says
/// which is which in its own words: *"the document holds both readings and
/// the exploitable one is the one written as the rule."* Its stated fix is
/// one conjunct — *"…and the read is not a `Crossing::Lift`, `Command` or
/// `Store`"* — and R2's status line is *"**BREAK**, one-clause fix, not
/// applied"*. It is applied here. `Command` has no read-side
/// [`crate::Crossing`] variant to name; a command argument is not a signal
/// read, and the value written by one is A3's business.
pub struct Writers {
    written: BTreeSet<DefId>,
    /// Where each signal is first written, for the diagnostic.
    at: BTreeMap<DefId, Span>,
}

impl Writers {
    pub fn of(hir: &Hir, split: &TierSplit) -> Writers {
        let mut written = BTreeSet::new();
        let mut at = BTreeMap::new();
        for (id, _) in hir.defs.iter() {
            for site in sites_of(hir, id) {
                let (signal, span) = match site {
                    Site::Write { signal, span, .. } => (signal, span),
                    // The keystroke write. This arm is the R2 repair.
                    Site::Bind { signal, span, .. } => (signal, span),
                    Site::Call { .. }
                    | Site::ForeignCall { .. }
                    | Site::Read { .. }
                    | Site::NotAPlace { .. }
                    | Site::Environment { .. }
                    // A request writes no cell either: it *is* a cell's
                    // initialiser, and what that cell is worth is decided
                    // by `flow` above rather than by the written set.
                    | Site::Outbound { .. }
                    // A capability reads the filesystem and writes no
                    // cell, so it puts no signal in the written set. A
                    // media query reads the display and writes none
                    // either — what makes a `remembered` cell externally
                    // written is its placement, decided below.
                    | Site::Media { .. }
                    | Site::Scroll { .. }
                    | Site::Build { .. }
                    // A document key handler writes nothing by existing.
                    // Whatever its *body* writes is a `Site::Write` of its
                    // own, recorded by the same walk one line later, so
                    // the writer is visible here under its own statement
                    // and G-SIG clause 2 is not silently widened.
                    | Site::DocumentKey { .. } => continue,
                };
                written.insert(signal);
                at.entry(signal).or_insert(span);
            }
        }
        // §21.8.4's `Store` conjunct. The question — *can anything outside
        // this program's statement forms put a value in the cell?* — is
        // `SignalPlacement::is_externally_written`, stated positively over
        // the placement for the reason `may_be_secret` is, and asked here
        // rather than answered here so the two enforcement sites cannot
        // drift from the classification.
        //
        // **`remembered` is a fifth placement that answers yes**, and it is
        // the one the conjunct was written for without knowing it. A
        // `durable` cell is externally writable because a previous
        // deployment is not in this program; a `remembered` cell is
        // externally writable because a previous *visit* is not either,
        // and neither is another tab, and neither is any other script on
        // the origin. `starting "light"` describes the value on a browser
        // that has never run this program, and on no other, so G-SIG's
        // second clause — *no write site, therefore it holds its
        // initialiser* — has a premise that is false here even when the
        // program contains no `set` at all.
        //
        // This is the anti-laundering rule, and it is one line because the
        // pass was built to have somewhere for it to go.
        for (id, def) in hir.defs.iter() {
            let DefKind::Signal(signal) = &def.kind else {
                continue;
            };
            if zdc_types::SignalPlacement::from_ast(signal.placement).is_externally_written() {
                written.insert(id);
                at.entry(id).or_insert(def.span);
            }
            // **The clock conjunct.** The same shape as `Site::Bind`'s and
            // for the same reason: the browser puts a value in the cell and
            // there is no `set` anywhere for a reachability query over
            // statement forms to find, so G-SIG clause 2 would grant this
            // Trusted on the strength of a resting `0` nothing ever reads.
            //
            // Wrong, and not only pedantically. A clock reading is
            // *environmental*: the wall clock is whatever the visitor's
            // machine says it is, so a decision derived from elapsed time
            // is derived from a value the visitor can move. §21.9 settled
            // exactly this against the prelude's own `clock` — the impure
            // primitive that made `gives pure T` necessary — and a signal
            // that reads time by declaration rather than by call has to get
            // the same answer, or the two spellings of "what time is it"
            // disagree about who is allowed to trust it.
            //
            // **A schedule is the same conjunct** (§14G.4, #18). A
            // scheduled cell holds the beat's start time, which the
            // deployment's scheduler puts there; the `0` on the
            // declaration is a resting value nothing ever reads, exactly
            // as a clock's is, so G-SIG clause 2 would otherwise award a
            // platform timestamp the authority of a literal.
            //
            // The near miss is worth naming, because reasoning the other
            // way is easy. The *cadence* really is as trusted as the
            // source text: this compiler generated the cron rule from it.
            // The **time** is not the cadence. It is a clock reading, and
            // §21.9 settled that a clock reading is not evidence — which
            // is what made `gives pure T` necessary in the first place —
            // so a beat cannot be worth more than `clock` is.
            if signal.clock.is_some() || signal.schedule.is_some() {
                written.insert(id);
                at.entry(id).or_insert(def.span);
            }
        }
        // §21.8.4's `Lift` conjunct.
        for lifted in split.lifted.values() {
            for signal in lifted {
                written.insert(*signal);
                at.entry(*signal).or_insert(hir.defs[*signal].span);
            }
        }
        Writers { written, at }
    }

    pub fn is_written(&self, signal: DefId) -> bool {
        self.written.contains(&signal)
    }

    pub fn written_at(&self, signal: DefId) -> Option<Span> {
        self.at.get(&signal).copied()
    }
}

/// The integrity of an expression, as a total function over `HirExprKind`.
///
/// Every arm is written out. There is no wildcard anywhere in this
/// function and there must never be one: a new expression form has to be
/// ruled on here, and the compiler is what makes that happen. That
/// property is the entire argument that the grant set is closed — §19.5's
/// completeness claim is a claim about the grammar, and it survives only
/// while this match is exhaustive by construction.
pub struct Integrity<'a> {
    hir: &'a Hir,
    /// Fixpoint 1's answers, so that a signal read and a call are table
    /// lookups rather than a walk into another definition's body.
    ///
    /// Before the table existed, a read of a signal re-derived that
    /// signal's initialiser at every reference and descended a chain of
    /// derived signals once per read — the same answer computed `n` times
    /// down a chain of length `n`, with nothing stopping a cycle. That is
    /// the interprocedural half of this analysis, and it belongs in a
    /// fixpoint rather than in an expression walk.
    solution: &'a Solution,
    /// What each binding in the body being walked is worth, as a [`Flow`]
    /// over the enclosing definition's parameters. Absent means Untrusted,
    /// because absent means no grant applies.
    locals: BTreeMap<LocalId, Flow>,
}

impl<'a> Integrity<'a> {
    pub fn new(hir: &'a Hir, solution: &'a Solution) -> Integrity<'a> {
        Integrity {
            hir,
            solution,
            locals: BTreeMap::new(),
        }
    }

    /// Bind `local` for the duration of one body's walk.
    ///
    /// A parameter is bound to [`Flow::param`] while its own definition is
    /// being summarised, and to [`Flow::exact`] once every call site's
    /// argument has been merged onto it. Those are the two modes, and they
    /// are the same walk.
    pub fn bind(&mut self, local: LocalId, flow: Flow) {
        self.locals.insert(local, flow);
    }

    pub fn clear_bindings(&mut self) {
        self.locals.clear();
    }

    /// What a **read** of a signal is worth — G-SIG, both clauses.
    ///
    /// Exposed separately from [`Integrity::of`] because G-SIG is a
    /// question about a declaration rather than about an expression, and
    /// the obligation sites (A1, A3) ask it directly.
    pub fn of_signal_read(&self, signal: DefId) -> Authority {
        self.solution.signal(signal).0
    }

    /// The authority of an expression, and the grant that awarded it.
    ///
    /// `None` for the grant means no grant applied, which under a closed
    /// lattice is the ordinary case and yields [`Authority::Untrusted`].
    pub fn of(&self, expr: ExprId) -> (Authority, Option<Grant>) {
        let (flow, grant) = self.flow(expr);
        (flow.authority(), grant)
    }

    /// The authority of an expression as a function of the enclosing
    /// definition's parameters, and the grant that awarded it.
    ///
    /// This is the total function. Every arm is written out. There is no
    /// wildcard anywhere in it and there must never be one: a new
    /// expression form has to be ruled on here, and the compiler is what
    /// makes that happen.
    pub fn flow(&self, expr: ExprId) -> (Flow, Option<Grant>) {
        match &self.hir.exprs[expr].kind {
            // G-LIT. The grammar is the check.
            HirExprKind::Number(_)
            | HirExprKind::Text(_)
            | HirExprKind::Truth(_)
            | HirExprKind::Empty => (Flow::trusted(), Some(Grant::Literal)),

            // G-ENV. The operator set the variable, not a visitor.
            HirExprKind::Environment(_) => (Flow::trusted(), Some(Grant::Environment)),

            // `address` is the URL a visitor asked for. No grant covers
            // it, so the default answers: Untrusted, and the closed set is
            // why this needs no argument about whether the enumeration of
            // untrusted sources is complete. There is no enumeration.
            HirExprKind::Address => (Flow::untrusted(), None),
            // No grant. The visitor chose the system theme, the window
            // size and whether animation is wanted, so the answer is
            // attacker-chosen in exactly the sense this lattice means:
            // whoever is at the browser decided it. Untrusted needs no
            // arm of its own under a closed set, but the match is total
            // and it is written out so the decision is on the record.
            HirExprKind::Media(_) => (Flow::untrusted(), None),

            // Untrusted, and the reason is the same one the clock carries:
            // a visitor controls their own scrollbar. A reading is
            // environmental — whoever is at the browser decided it — so a
            // program that let one reach a `trusted` sink would be
            // endorsing a number the reader chose.
            HirExprKind::Scroll => (Flow::untrusted(), None),

            // **A response body is Untrusted, and nothing can make it
            // anything else** (#19). It is the answer of a host the
            // program named and nobody else vouches for, so no grant in
            // [`Grant::CLOSED_LIST`] describes it — and because this
            // lattice is default-closed rather than default-open, saying
            // so costs one arm and no argument about whether an
            // enumeration of untrusted sources is complete.
            //
            // The arguments are deliberately **not** joined in. A join
            // would be the shape `Grant::ForeignPure` has, and it would
            // say the answer is a function of what was sent — which is
            // exactly the claim a third party's server is under no
            // obligation to honour. `Flow::untrusted()` is a constant
            // here, so a request sent with nothing but literals still
            // comes back Untrusted.
            HirExprKind::Outbound { .. } => (Flow::untrusted(), None),

            // G-BLD, and the one arm of this function that had a grant
            // waiting for it. See [`Grant::Build`].
            HirExprKind::Build {
                capability,
                argument,
            } => self.of_build(*capability, *argument),

            // A composite is the join of its parts, and carries no grant of
            // its own: joining is the only way authority moves.
            // The condition joins too: which arm is taken is decided by
            // it, so a value chosen by untrusted input is untrusted
            // however trusted both arms are.
            HirExprKind::Conditional {
                condition,
                value,
                otherwise,
            } => (
                self.flow(*condition)
                    .0
                    .join(&self.flow(*value).0)
                    .join(&self.flow(*otherwise).0),
                None,
            ),
            HirExprKind::List(items) => (
                items
                    .iter()
                    .fold(Flow::trusted(), |acc, item| acc.join(&self.flow(*item).0)),
                None,
            ),
            HirExprKind::Map(pairs) => (
                pairs.iter().fold(Flow::trusted(), |acc, (key, value)| {
                    acc.join(&self.flow(*key).0).join(&self.flow(*value).0)
                }),
                None,
            ),
            HirExprKind::Unary { operand, .. } => (self.flow(*operand).0, None),
            HirExprKind::Operator { operand, .. } => (self.flow(*operand).0, None),
            HirExprKind::Binary { lhs, rhs, .. } => {
                (self.flow(*lhs).0.join(&self.flow(*rhs).0), None)
            }
            HirExprKind::Field { base, .. } => (self.flow(*base).0, None),
            // An index joins the indexed value with the index itself: a
            // browser that chooses `i` chooses which element comes out,
            // which is the whole of obligation site A1.
            HirExprKind::Index { base, index } => {
                (self.flow(*base).0.join(&self.flow(*index).0), None)
            }
            // The longer list holds everything the shorter one held and
            // the item as well, so it carries the provenance of both. A
            // rule that took only the list's label would be a laundry:
            // `append attackerText to trusted` would come out trusted.
            HirExprKind::Append { item, list } => {
                (self.flow(*item).0.join(&self.flow(*list).0), None)
            }
            // And the bigger map holds everything the smaller one held
            // plus the entry, so it carries the provenance of all three
            // operands, for the reason `append` carries both of its.
            HirExprKind::Insert { key, value, table } => (
                self.flow(*key)
                    .0
                    .join(&self.flow(*value).0)
                    .join(&self.flow(*table).0),
                None,
            ),
            // The container that comes out holds whatever the body made
            // of the payload, and it is still the same `None`/`Loading`/
            // `Failed` when it was one of those — so it carries the
            // provenance of both, for the reason `append` carries both of
            // its. A rule that took only the body's would make
            // `map each x in attackerValue to x` trusted.
            HirExprKind::MapInside { source, to, .. } => {
                (self.flow(*source).0.join(&self.flow(*to).0), None)
            }

            HirExprKind::Ref(res) => self.of_res(*res),

            HirExprKind::Call { callee, args } => self.of_call(*callee, args),
            HirExprKind::OfCall { callee, operand } => {
                self.of_call(*callee, &[HirArg::Positional(*operand)])
            }
        }
    }

    /// One `build` capability — §4.4's closed set, ruled on per member.
    ///
    /// Written out rather than answered once for `build`, because the
    /// capabilities are not all the same kind of thing and answering them
    /// together is how §21.8 says a grant table goes wrong.
    fn of_build(&self, capability: BuildCapability, argument: ExprId) -> (Flow, Option<Grant>) {
        match capability {
            // **G-BLD.** A path the author wrote, resolved against the
            // project directory before it is opened, naming a file in the
            // operator's own version control. A path from anywhere else
            // names whatever chose it, and no grant covers that.
            BuildCapability::Read | BuildCapability::List => {
                if matches!(self.hir.exprs[argument].kind, HirExprKind::Text(_)) {
                    (Flow::trusted(), Some(Grant::Build))
                } else {
                    (Flow::untrusted(), None)
                }
            }
            // Not a read at all, and so not G-BLD: `build markdown` renders
            // the text it is handed, in this compiler, deterministically.
            // It is the one capability whose result really *is* a function
            // of its argument — which is the assumption §18.1 semantics 6
            // made about a `foreign` and §19.11 refuted, sound here for the
            // reason it was unsound there: the implementation is the
            // compiler rather than somebody's JavaScript. So it propagates
            // and grants nothing, and markdown rendered from a
            // browser-chosen string stays exactly as authored as that
            // string was.
            //
            // `build parts` is the same kind of thing and is ruled on the
            // same way: it renders its prose with that renderer and splits
            // on a fence this compiler owns, so it too is a function of
            // its argument alone. It grants nothing in particular because
            // a widget *name* is the one thing it produces that selects
            // something, and that name is checked against the program's
            // own `choice Widget` before the value exists at all — an
            // authority question settled by refusal rather than by a
            // label.
            BuildCapability::Markdown | BuildCapability::Parts => (self.flow(argument).0, None),
        }
    }

    /// A resolved name.
    fn of_res(&self, res: Res) -> (Flow, Option<Grant>) {
        match res {
            // A binder carries whatever was bound to it and no grant of its
            // own. In particular an endorsed release parameter is **not**
            // Trusted inside the body: §19.10.3(a) makes the endorsement
            // site-local and result-transparent, and raising the label
            // inside would turn four lines into a universal integrity
            // launderer. G-REL is awarded at the call site, by A5.
            Res::Local(local) => (self.locals.get(&local).cloned().unwrap_or_default(), None),
            Res::Def(def) => self.of_def(def),
            // A variant name is a constructor written in the source, so it
            // is as trusted as a literal is. Its *arguments* are joined by
            // the `Call` arm above; this is the bare name.
            Res::Variant { .. } | Res::BuiltinVariant(_) => (Flow::trusted(), Some(Grant::Literal)),
            // An element, a type name or the pair constructor is not a
            // value a browser can choose. The pair's *arguments* are
            // joined by the `Call` arm above, exactly as a variant's are;
            // this is the bare name.
            Res::Builtin(Builtin::Element(_) | Builtin::Type | Builtin::Pair) => {
                (Flow::trusted(), Some(Grant::Literal))
            }
        }
    }

    /// A reference to a top-level definition.
    fn of_def(&self, def: DefId) -> (Flow, Option<Grant>) {
        match &self.hir.defs[def].kind {
            // G-SIG, both clauses — solved once, in fixpoint 1, because
            // clause 2 reads the initialiser and an initialiser may call a
            // function whose body reads another signal.
            DefKind::Signal(_) => {
                let (authority, grant) = self.solution.signal(def);
                (Flow::exact(authority), grant)
            }
            // A bare reference to a callable is not a value in this
            // language; its result is settled at the call site.
            DefKind::Function(_)
            | DefKind::Foreign(_)
            | DefKind::Release(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => (Flow::untrusted(), None),
        }
    }

    /// The result of calling `callee`.
    fn of_call(&self, callee: Res, args: &[HirArg]) -> (Flow, Option<Grant>) {
        let joined = args.iter().fold(Flow::trusted(), |acc, arg| {
            acc.join(&self.flow(arg_of(arg)).0)
        });
        let Res::Def(def) = callee else {
            // A builtin constructor such as `Some with value is v` is as
            // trusted as what it wraps.
            return (joined, None);
        };
        match &self.hir.defs[def].kind {
            // **§21.9.** The result grant is read; `foreign.site` is not,
            // and must not be. A placement answers which bundles a library
            // may be linked into, and no answer to that question can
            // establish that a result is a function of the arguments —
            // which is R1, in one line, at the site that had it wrong.
            //
            // Exhaustive over [`zdc_ast::ForeignGrant`], with no wildcard,
            // so a fourth claim about a result has to be ruled on here.
            DefKind::Foreign(foreign) => match foreign.result_grant {
                // G-FGN-T. Unconditional, and unconditionally a human's
                // word (R5).
                zdc_ast::ForeignGrant::Trusted => (Flow::trusted(), Some(Grant::ForeignTrusted)),
                // G-FGN-P. The join-of-arguments rule, now conditional on
                // the marker it always needed. A pure foreign of no
                // arguments joins `∅` and is Trusted, which is correct:
                // such a function is a constant.
                zdc_ast::ForeignGrant::Pure => (joined, Some(Grant::ForeignPure)),
                // No marker, no grant — whatever the placement says. For
                // all the compiler knows this reads the wall clock or the
                // request URL, and `clock` and `queryParam` are both here.
                zdc_ast::ForeignGrant::Opaque => (Flow::untrusted(), None),
            },
            // Interprocedural, and this is the whole point of the summary:
            // the result is whatever the body computes, instantiated at
            // *this* call site's arguments. A function whose body reads an
            // Untrusted signal is Untrusted however Trusted its arguments
            // were, which the old "a function is transparent" rule got
            // backwards.
            //
            // A release's summary additionally carries every parameter,
            // because §19.2 rule 3 makes its result the join of its
            // arguments as written at the call site, before endorsement.
            DefKind::Function(function) => {
                let params = function.params.clone();
                (self.instantiate(def, &params, args), None)
            }
            DefKind::Release(release) => {
                let params = release.params.clone();
                (self.instantiate(def, &params, args), None)
            }
            DefKind::Signal(_)
            | DefKind::View(_)
            | DefKind::Record(_)
            | DefKind::Choice(_)
            | DefKind::Component(_) => (joined, None),
        }
    }

    /// A callable's summary, instantiated at one call site's arguments.
    fn instantiate(&self, callee: DefId, params: &[LocalId], args: &[HirArg]) -> Flow {
        let matched = match_args(self.hir, params, args);
        let flows: Vec<Flow> = matched
            .iter()
            .map(|arg| match arg {
                Some(expr) => self.flow(*expr).0,
                None => Flow::untrusted(),
            })
            .collect();
        self.solution.result(callee).substitute(&flows)
    }
}

/// The expression an argument carries, named or positional.
fn arg_of(arg: &HirArg) -> ExprId {
    crate::sites::arg_expr(arg)
}

/// **REL-PURE** — §21.7.3 as amended by §21.9, error **E-REL-10**.
///
/// A release body may reach only a `foreign` that declares `gives pure T`
/// or `gives trusted T`. Checked at the declaration and transitive over the
/// call graph, exactly as REL-CLOSED is.
///
/// **What §21.9 changed, and what it did not.** The rule used to demand
/// `is anywhere`, which classifies linkability rather than purity — so the
/// prelude's own `clock` passed it while reading the wall clock, and
/// §21.8.1's `queryParam` passed it while reading the request URL. It now
/// demands the marker built for the question, and both are refused.
///
/// The marker is a human's word about JavaScript the compiler cannot read.
/// This rule therefore reports what a declaration **says**, and nothing
/// about what the JavaScript does. It must not be described to a user as
/// establishing that a release body is pure.
pub fn rel_pure(hir: &Hir, release: DefId) -> Vec<GraphError> {
    let mut out = Vec::new();
    for (foreign, span) in reachable_foreigns(hir, release) {
        let DefKind::Foreign(decl) = &hir.defs[foreign].kind else {
            continue;
        };
        // Exhaustive, so a fourth result grant has to be ruled on here
        // rather than defaulting into the rule on either side.
        match decl.result_grant {
            zdc_ast::ForeignGrant::Pure | zdc_ast::ForeignGrant::Trusted => continue,
            zdc_ast::ForeignGrant::Opaque => {}
        }
        let name = hir.defs[foreign].name.clone();
        let where_it_runs = match decl.site {
            zdc_ast::ForeignSite::Client => "is client",
            zdc_ast::ForeignSite::Server => "is server",
            zdc_ast::ForeignSite::Anywhere => "is anywhere",
        };
        out.push(
            GraphError::new(
                "E-REL-10",
                // §21.6 item 18 forbids this text from saying *"a release
                // body may observe nothing but its parameters"*: §21.8.6
                // item 6 adds `visitor` as a thing a body observes, and the
                // marker below is asserted rather than checked. The rule
                // states what it requires, and stops.
                format!(
                    "`{}` reaches the foreign `{name}`, whose `gives` line declares neither \
                     `pure` nor `trusted`. Rule REL-PURE (§21.7.3, amended §21.9).",
                    hir.defs[release].name
                ),
                hir.defs[release].span,
            )
            .with_notes(vec![(span, format!("`{name}` is reached here"))])
            .with_help(format!(
                "`{name}` is declared `{where_it_runs}`, which answers which bundles it may be \
                 linked into and says nothing about its result. Write `gives pure T` to declare \
                 that the result is a function of the arguments — a claim about the JavaScript \
                 that nobody checks — or `gives trusted T` to sign that the result is not \
                 attacker-chosen, or lift the value into the release's parameter list where an \
                 endorsement has to name it."
            )),
        );
    }
    out
}

/// **REL-CLOSED** — §19.2 rule 8, error **E-REL-04**.
///
/// A release body, and everything it reaches, may read no signal. This is
/// what makes the parameter list the release's entire input, and it is the
/// premise REL-ARG never had in §19.10.
pub fn rel_closed(hir: &Hir, release: DefId) -> Vec<GraphError> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = vec![release];
    while let Some(def) = queue.pop() {
        if !seen.insert(def) {
            continue;
        }
        for site in sites_of(hir, def) {
            match site {
                Site::Read { signal, span, .. } => {
                    let name = hir.defs[signal].name.clone();
                    out.push(
                        GraphError::new(
                            "E-REL-04",
                            format!(
                                "`{}` reads the signal `{name}`. A release's inputs are its \
                                 parameters and nothing else.",
                                hir.defs[release].name
                            ),
                            span,
                        )
                        .with_help(
                            "Pass the signal's value as an argument, where the call site has to \
                             account for where it came from."
                                .to_string(),
                        ),
                    );
                }
                Site::Call { callee, .. } => queue.push(callee),
                // A foreign call is REL-PURE's business, not REL-CLOSED's.
                Site::ForeignCall { .. }
                | Site::Write { .. }
                | Site::Bind { .. }
                | Site::NotAPlace { .. }
                | Site::Environment { .. }
                // None of these is a signal read, and none can occur in a
                // release body at all: a release runs in `Region::Server`,
                // where the split raises E0361 for a capability, E0362 for
                // a media query and E0363 for a request. REL-CLOSED has
                // nothing left to say about any of them.
                | Site::Media { .. }
                | Site::Scroll { .. }
                | Site::Build { .. }
                | Site::Outbound { .. }
                // A release has no nodes, so it has no handler; and were
                // one reachable, E0364 refuses it for the same reason as
                // the three above — a release runs in `Region::Server`,
                // which has no document.
                | Site::DocumentKey { .. } => {}
            }
        }
    }
    out
}

/// Every `foreign` reachable from a definition's body, with the span that
/// reaches it. Transitive over calls, as REL-PURE requires.
///
/// Visible to [`crate::report`] as well as to [`rel_pure`], because the two
/// want opposite answers from one walk: the rule asks *which foreigns does
/// this release reach that it may not*, and the report asks *which releases
/// reach this grant*. Deriving the second from a second walk is how the two
/// come to disagree about the call graph.
pub(crate) fn reachable_foreigns(hir: &Hir, from: DefId) -> Vec<(DefId, Span)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = vec![from];
    while let Some(def) = queue.pop() {
        if !seen.insert(def) {
            continue;
        }
        for site in sites_of(hir, def) {
            match site {
                Site::ForeignCall { callee, span } => out.push((callee, span)),
                // Transitive: REL-PURE is checked over the call graph,
                // exactly as REL-CLOSED is.
                Site::Call { callee, .. } => queue.push(callee),
                Site::Read { .. }
                | Site::Write { .. }
                | Site::Bind { .. }
                | Site::NotAPlace { .. }
                | Site::Environment { .. }
                // None of these is supplied by a `foreign` declaration —
                // one comes from the compiler, one from the browser, and a
                // key handler is not a call at all, and a request is the
                // runtime's own `fetch` with no program-declared module on
                // the path — so none names a library for REL-PURE to ask
                // about.
                | Site::Media { .. }
                | Site::Scroll { .. }
                | Site::Build { .. }
                | Site::Outbound { .. }
                | Site::DocumentKey { .. } => {}
            }
        }
    }
    out
}

/// **W-REL-01** — §19.4. An unbounded release.
///
/// The `gives` type is the per-evaluation bandwidth, and with no `limit`
/// there is no per-session ceiling on evaluations either.
///
/// The warning deliberately does **not** say that adding a `limit` bounds
/// disclosure, because it does not: `limit` is per declaration and per
/// anonymous session, k declarations give kN, and clearing a cookie mints
/// a fresh budget (§21.8.7, residual risk R3).
///
/// **And nothing counts them.** The clause this warning asks for is read
/// by four consumers — this warning, `zdc-doc`'s page, `zdc-lsp`'s hover,
/// and `zdc-types`, where §19.2 rule 5 makes a budgeted call `Option of T`
/// so exhaustion cannot be forgotten. It reaches no line of
/// `zdc-codegen`, `zdc-host` or `zdc-store`, so the `Option`'s exhausted
/// case is a variant the emitted program never produces. The help below
/// used to end *"until durable storage exists"*, which was true when it
/// was written and stopped being true when `zdc-store` landed: durable
/// storage exists and the budget is still wired to nothing, which is a
/// stronger statement than the conditional it replaced and the one #212
/// leans on.
pub fn w_rel_01(hir: &Hir, release: DefId) -> Option<GraphError> {
    let DefKind::Release(decl) = &hir.defs[release].kind else {
        return None;
    };
    if decl.limit.is_some() {
        return None;
    }
    Some(
        GraphError::warning(
            "W-REL-01",
            format!(
                "`{}` has no `limit`, so one session may evaluate it any number of times.",
                hir.defs[release].name
            ),
            hir.defs[release].span,
        )
        .with_help(
            "Writing `limit N per visitor` states a cap on evaluations of this one declaration \
             against one anonymous session, and nothing counts them: no budget is emitted, so \
             the exhausted case of the `Option` a `limit` gives the call site never arrives. It \
             is not a cumulative disclosure bound either — a second declaration carries its own \
             budget, and clearing a cookie mints a fresh one."
                .to_string(),
        ),
    )
}

/// **E-INT-01** — §18.1 semantics 9, as narrowed by §21.7.3. `trusted`
/// on a placement that cannot carry it.
///
/// The one rule here that reads no label at all, and the only piece of
/// the default-open pass this module replaced that had nothing in the
/// closed design to be subsumed by. The closed lattice took over that
/// pass's A1 and A3 obligations, and took over its enumeration of
/// untrusted sources by not needing one — but a declaration rule is not a
/// labelling rule.
///
/// **Two of its three arms did not survive the move, and neither is an
/// oversight:**
///
/// * `trusted static` was E-INT-01 because `static` was already trusted.
///   §21.7.3 deletes that blanket grant — a build that fetches a feed
///   through an ungranted `foreign` produces Untrusted `static` state —
///   so the word is now exactly what a declaration is for, and the spec
///   says in as many words that it *must no longer be E-INT-01*.
/// * `trusted` on a derived (`from`) signal was E-INT-01 because nothing
///   writes it and `trusted` was only an obligation. Under G-SIG clause 1
///   the declaration is a **grant**, so it is meaningful on a derived
///   signal too: it endorses a derivation the lattice would otherwise
///   call Untrusted.
///
/// What is left is the arm that is about the placement rather than about
/// the lattice: a browser owns its own memory, so there is no such thing
/// as protecting one from itself.
pub fn int_01(hir: &Hir) -> Vec<GraphError> {
    let mut out = Vec::new();
    for (_, def) in hir.defs.iter() {
        let DefKind::Signal(signal) = &def.kind else {
            continue;
        };
        if !signal.trusted {
            continue;
        }
        let name = def.name.clone();
        match signal.placement {
            zdc_ast::Placement::Client => out.push(
                GraphError::new(
                    "E-INT-01",
                    format!(
                        "`{name}` is `trusted client`, and a browser owns its own memory. There \
                         is no such thing as protecting a browser from itself, so `client` state \
                         cannot be trusted."
                    ),
                    def.span,
                )
                .with_help(
                    "Declare it `server` or `durable` if the point is that no browser may choose \
                     what goes in it."
                        .to_string(),
                ),
            ),
            // The same rule as `client` above and a strictly stronger
            // reason for it, so it is a separate arm with its own
            // sentence rather than an addition to that or-pattern.
            //
            // A `trusted client` declaration is refused because a browser
            // owns its own memory. A `trusted remembered` declaration is
            // refused because the browser owns its own *disk*, and
            // because — unlike memory, which dies with the tab — this
            // cell keeps whatever was put in it across visits. Allowing
            // the word here would be the whole laundering attack in one
            // declaration: write a literal today, read it back Trusted
            // tomorrow, having never once been told what happened to it
            // in between.
            zdc_ast::Placement::Remembered => out.push(
                GraphError::new(
                    "E-INT-01",
                    format!(
                        "`{name}` is `trusted remembered`, and any other script on the origin \
                         may write a `remembered` value, so the program cannot promise what is \
                         in it. `zdc explain E-INT-01`."
                    ),
                    def.span,
                )
                .with_help(
                    "Declare it `server` or `durable` if the point is that no browser may \
                     choose what goes in it. A `remembered` value is a preference the visitor \
                     is entitled to change, and reading one is always Untrusted."
                        .to_string(),
                ),
            ),
            zdc_ast::Placement::Static
            | zdc_ast::Placement::Server
            | zdc_ast::Placement::Durable => {}
        }
    }
    out
}
