//! Regions, roots and contexts — spec §17.2.3.
//!
//! Three regions, not five. `durable` is storage: §5.5 forbids a derived
//! durable signal, so durable state has no code of its own, and §5.2's
//! same-domain clause already groups server and durable. `durable per
//! visitor` is a durable key with a session prefix, not a region.

use zdc_ast::Placement;
use zdc_hir::DefId;
use zdc_lexer::Span;
use zdc_types::{ReadContext, SignalPlacement};

/// Where code runs. Placement is a property of *state*; a region is a
/// property of *code*, and it is inherited from the root the code was
/// reached from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Region {
    Static,
    Client,
    Server,
}

/// What started the walk that reached this code.
///
/// The distinction is not decorative: §14G.1.4 gives a view-rooted server
/// derivation and a trigger-rooted one different read tables, because one
/// has a browser attached and the other does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootKind {
    Build,
    View,
    Trigger,
}

/// A region paired with the kind of root it was reached from. At most five
/// of the nine combinations inhabit a program (§17.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ctx {
    pub region: Region,
    pub kind: RootKind,
}

impl Ctx {
    pub const CLIENT_VIEW: Ctx = Ctx {
        region: Region::Client,
        kind: RootKind::View,
    };
    pub const CLIENT_TRIGGER: Ctx = Ctx {
        region: Region::Client,
        kind: RootKind::Trigger,
    };
    pub const STATIC_BUILD: Ctx = Ctx {
        region: Region::Static,
        kind: RootKind::Build,
    };
    pub const SERVER_VIEW: Ctx = Ctx {
        region: Region::Server,
        kind: RootKind::View,
    };
    pub const SERVER_TRIGGER: Ctx = Ctx {
        region: Region::Server,
        kind: RootKind::Trigger,
    };

    /// Every context that can inhabit a program, in a fixed order, so a
    /// test can assert the mapping onto [`ReadContext`] is total.
    pub const ALL: [Ctx; 5] = [
        Ctx::STATIC_BUILD,
        Ctx::CLIENT_VIEW,
        Ctx::CLIENT_TRIGGER,
        Ctx::SERVER_VIEW,
        Ctx::SERVER_TRIGGER,
    ];

    /// §17.2.3's mapping onto the type checker's vocabulary. The two
    /// client rows of §14G.1.4 are identical, so [`ReadContext`] is
    /// exactly the type-relevant quotient of this type.
    pub fn read_context(self) -> ReadContext {
        match (self.region, self.kind) {
            (Region::Static, _) => ReadContext::Static,
            (Region::Client, _) => ReadContext::Client,
            (Region::Server, RootKind::Trigger) => ReadContext::TriggerRootedServer,
            (Region::Server, _) => ReadContext::ViewRootedServer,
        }
    }

    pub fn describe(self) -> &'static str {
        match (self.region, self.kind) {
            (Region::Static, _) => "build-time evaluation",
            (Region::Client, RootKind::Trigger) => "a client-placed trigger",
            (Region::Client, _) => "the browser",
            (Region::Server, RootKind::Trigger) => "a trigger handler",
            (Region::Server, _) => "a server invocation the view asked for",
        }
    }
}

/// Which region a placement's code runs in.
pub fn region_of(placement: SignalPlacement) -> Region {
    match placement {
        // The store is the browser's, so the code that reads and writes it
        // runs where the browser is. There is no second machine involved
        // at any point, which is the whole difference from `durable`.
        SignalPlacement::Client | SignalPlacement::Remembered => Region::Client,
        SignalPlacement::Static => Region::Static,
        SignalPlacement::Server | SignalPlacement::Durable | SignalPlacement::DurablePerVisitor => {
            Region::Server
        }
    }
}

pub fn placement_of(placement: Placement) -> SignalPlacement {
    SignalPlacement::from_ast(placement)
}

/// Which mutation operator a command performs. Five fixed words, so
/// §17.2.5 fatal 3's injectivity argument holds: the operator occupies
/// exactly position 2 of a rendered command name, and all five are
/// reserved words (§4.2) that no record field can spell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MutOp {
    Set,
    Incr,
    Decr,
    /// `append v to xs` — §14B.2's membership verbs.
    Append,
    /// `remove v from xs`.
    Remove,
}

impl MutOp {
    pub fn word(self) -> &'static str {
        match self {
            MutOp::Set => "set",
            MutOp::Incr => "incr",
            MutOp::Decr => "decr",
            MutOp::Append => "append",
            MutOp::Remove => "remove",
        }
    }
}

/// A path segment as it appears in a command name. An index renders as
/// `.at` whatever it indexes by, because the *value* of the index is a
/// runtime argument, not part of the endpoint's identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathKeySeg {
    Index,
    Field(String),
}

/// What identifies a generated command endpoint — §17.2.5 fatal 3.
///
/// The path is part of the key *and* part of the rendered name. Keying on
/// the path but naming without it produced two roots with one name and one
/// emitted file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandKey {
    pub signal: DefId,
    pub op: MutOp,
    pub path: Vec<PathKeySeg>,
}

impl CommandKey {
    /// §17.2.5 fatal 3's `name`. Injective over any set of keys: the
    /// operator is drawn from three fixed words and occupies position 2,
    /// and `at` is a reserved word (§4.2) so no record field renders as
    /// `.at`.
    pub fn render(&self, signal_name: &str) -> String {
        let mut out = String::from(signal_name);
        out.push('.');
        out.push_str(self.op.word());
        for segment in &self.path {
            match segment {
                PathKeySeg::Index => out.push_str(".at"),
                PathKeySeg::Field(field) => {
                    out.push('.');
                    out.push_str(field);
                }
            }
        }
        out
    }
}

/// One mutation site, addressable past a statement index.
///
/// `ordinal` counts, in one pre-order traversal of the owner's body **and**
/// its view nodes, every mutation statement and every two-way binding. One
/// counter, one traversal, both kinds addressable (§17.2.5 fatal 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MutSite {
    pub owner: DefId,
    pub ordinal: u32,
}

/// Why a root exists.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootOrigin {
    /// Singleton, `(Client, View)`.
    ClientBundle,
    /// Singleton, `(Static, Build)`.
    BuildHost,
    /// On demand, from a `Remote` crossing.
    Endpoint(DefId),
    /// On demand, from a cross-region write.
    Command(CommandKey),
    /// One per `every`/`inbound` declaration.
    Trigger(DefId),
    /// §17.2.6 — checked, never emitted.
    Orphan(DefId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RootId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct Root {
    pub ctx: Ctx,
    pub origin: RootOrigin,
    pub span: Span,
    /// `false` only for [`RootOrigin::Orphan`]. That one boolean is what
    /// lets the pass be demand-driven for emission and total for
    /// diagnostics at the same time (§17.2.3).
    pub emitted: bool,
}

/// The two singleton roots, which always exist and may be empty.
pub const CLIENT: RootId = RootId(0);
pub const BUILD: RootId = RootId(1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_inhabitable_context_maps_onto_a_read_context() {
        // §17.2.3: the split's context has five values and `ReadContext`
        // has four, because the two client rows of §14G.1.4 are identical.
        let mapped: Vec<ReadContext> = Ctx::ALL.iter().map(|ctx| ctx.read_context()).collect();
        assert_eq!(mapped[1], mapped[2], "both client rows are the same row");
        let mut distinct = mapped.clone();
        distinct.sort_by_key(|c| c.describe());
        distinct.dedup();
        assert_eq!(distinct.len(), 4);
    }

    #[test]
    fn a_command_name_renders_its_whole_key() {
        // §17.2.5 fatal 3's worked examples, verbatim.
        let visits = DefId::from_index(0);
        use zdc_hir::ArenaId as _;
        assert_eq!(
            CommandKey {
                signal: visits,
                op: MutOp::Incr,
                path: vec![]
            }
            .render("visits"),
            "visits.incr"
        );
        assert_eq!(
            CommandKey {
                signal: visits,
                op: MutOp::Set,
                path: vec![PathKeySeg::Index, PathKeySeg::Field("done".into())]
            }
            .render("todos"),
            "todos.set.at.done"
        );
        assert_eq!(
            CommandKey {
                signal: visits,
                op: MutOp::Set,
                path: vec![PathKeySeg::Index]
            }
            .render("todos"),
            "todos.set.at"
        );
    }
}
