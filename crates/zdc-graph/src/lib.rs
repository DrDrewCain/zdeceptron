#![forbid(unsafe_code)]

//! Tier splitting and information flow — spec §17.
//!
//! These two passes are what make ZDeceptron a language rather than a
//! framework. Everything before them is true of any compiler; everything
//! after them depends on their answers.
//!
//! * **`split`** decides, for every definition, *which artifacts it is
//!   emitted into and what it means in each*. It reads the HIR and
//!   nothing else — no inference result anywhere — which is why it runs
//!   before the type checker rather than after (§17.1.1). The edge at
//!   which the client walk stops at a `server` read is simultaneously the
//!   dead-code cut and the `Remote of T` introduction, and that identity
//!   is what makes §14A.1's exclusion *provable* rather than heuristic.
//! * **`ifc`** decides whether any secret reaches any of §14G.1.3(c)'s six
//!   sinks, through data **or through control**. §5.3 claimed
//!   non-interference from a data-dependency analysis, which cannot see a
//!   branch outcome; §17.3 is the correction.
//!
//! The order is `split → zdc-types → ifc`, a DAG with no fixpoint between
//! passes (§17.1.2).

pub mod diag;
pub mod ifc;
pub mod label;
pub mod root;
pub mod sites;
pub mod split;

pub use crate::diag::{GraphError, Severity};
pub use crate::ifc::{ifc, Cleared, Sink, SinkSite, Verdict};
pub use crate::label::{Label, Obs, Secrecy};
pub use crate::root::{
    CommandKey, Ctx, MutOp, MutSite, PathKeySeg, Region, Root, RootId, RootKind, RootOrigin, BUILD,
    CLIENT,
};
pub use crate::sites::{sites_of, Site};
pub use crate::split::{
    classify, classify_write, split, unusable_path, BoundaryEdge, Crossing, Endpoint, EndpointKind,
    MemberForm, MutCrossing, TierSplit,
};

/// §17.1.4's re-export: `zdc-graph` speaks `zdc-types`' vocabulary at the
/// boundary, so the crate building the type checker changes as little as
/// possible.
pub use zdc_types::{ReadContext, ReadKind, SignalPlacement};
