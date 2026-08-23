//! `dist/report.json` — §19.5's audit trail, rendered.
//!
//! [`zdc_graph::report()`] decides *what* is in the trail and states what it
//! does not claim; this decides what the file looks like. The split is the
//! one the rest of this crate already draws: a `GraphError` is a finding
//! and [`crate::render`] is how one reaches a person.
//!
//! # Why here and not in `zdc-graph`
//!
//! Two things this file needs live on this side of the graph. A span is a
//! byte range into the *linked* program, so turning one into a file and a
//! line needs [`zdc_resolve::Linked`], which `zdc-graph` does not depend
//! on. And JSON escaping is already owned here, by [`crate::json`] — a
//! third copy of it in a third crate is a third set of escapes that can
//! disagree with the other two.
//!
//! # The shape
//!
//! ```json
//! {
//!   "version": 1,
//!   "grants": [                     // the closed set, §19.5
//!     {"code": "G-FGN-P", "asserted": true}
//!   ],
//!   "asserted": [                   // every entry a reviewer must read
//!     {
//!       "code": "G-FGN-P",
//!       "name": "queryParam",
//!       "gives": "pure",            // the modifier as written
//!       "is": "anywhere",           // the placement, reported not consulted
//!       "from": "./request.js",     // string | null
//!       "export": "queryParam",
//!       "primitive": false,         // a `zd:` module, or somebody's file
//!       "declaredAt": {…},
//!       "calls": [{…}],
//!       "reachedByReleases": [
//!         {"name": "digitOracle", "declaredAt": {…}, "reachedAt": {…}}
//!       ]
//!     }
//!   ],
//!   "anywhere": [                   // §21.8's third assertion, R5
//!     {
//!       "name": "spin",
//!       "from": "./spin.js",
//!       "export": "spin",
//!       "primitive": false,
//!       "declaredAt": {…},
//!       "calls": [{…}]
//!     }
//!   ],
//!   "endorsed": [                  // every `trusted p` clause, site A5
//!     {"release": "digitOracle", "parameter": "all", "declaredAt": {…}}
//!   ],
//!   "library": {                    // the prelude's, named not located
//!     "pure": ["bitAnd", …],
//!     "trusted": [],
//!     "anywhere": ["bitAnd", …]
//!   },
//!   "notClaimed": ["…"]             // zdc_graph::NOT_CLAIMED, verbatim
//! }
//! ```
//!
//! **There is no `attackerReachable` key**, and `notClaimed` says why in
//! the file rather than only in a comment. §19.5 as amended by §21.7.7
//! specified that field; §21.8.3 and §21.8.7 withdrew it, on two
//! independent grounds, and neither expired when this report landed:
//!
//! 1. It cannot be computed for the grants that matter. The flag is set by
//!    walking a grant's arguments back to a crossing, and a purity grant's
//!    channel is inside its JavaScript — §21.8.1's `queryParam` takes a
//!    string literal and reads `location.search`, so the walk terminates
//!    at a literal and answers `false` (residual risk R6).
//! 2. It reads as a verdict and would be a false one. §21.7.10 tells a
//!    user that if nothing is marked `attacker_reachable` then no visitor
//!    can steer any declassification; for `launder3.zd` that list is empty
//!    and a visitor steers it with a query string.
//!
//! What this file emits instead is the enumeration — complete by grammar,
//! which no configured taint tool can manage — with the releases each
//! assertion is load-bearing for. Not the claim laid over it.
//!
//! Unlike [`crate::json`] this is a single document rather than one object
//! per line: a report is written once, whole, after a build has succeeded,
//! so the streaming argument that shapes the diagnostic stream does not
//! apply and `JSON.parse` of the whole file is what a consumer wants.

use zdc_graph::integrity::Grant;
use zdc_graph::report::{AnywherePlacement, AssertedGrant, Endorsement, ReleaseReach, Report};
use zdc_lexer::Span;
use zdc_resolve::Linked;

use crate::json::{field, position, string};
use crate::printable;

/// The report as one JSON document, trailing newline included.
pub fn json(report: &Report, linked: &Linked) -> String {
    let mut out = String::from("{");
    field(&mut out, "version", "1");
    out.push(',');
    field(&mut out, "grants", &closed_set());
    out.push(',');
    let entries: Vec<String> = report
        .asserted
        .iter()
        .map(|grant| asserted(grant, linked))
        .collect();
    field(&mut out, "asserted", &format!("[{}]", entries.join(",")));
    out.push(',');
    let placed: Vec<String> = report
        .anywhere
        .iter()
        .map(|placement| anywhere(placement, linked))
        .collect();
    field(&mut out, "anywhere", &format!("[{}]", placed.join(",")));
    out.push(',');
    let endorsed: Vec<String> = report
        .endorsed
        .iter()
        .map(|clause| endorsement(clause, linked))
        .collect();
    field(&mut out, "endorsed", &format!("[{}]", endorsed.join(",")));
    out.push(',');
    field(&mut out, "library", &library(&report.library));
    out.push(',');
    let not_claimed: Vec<String> = zdc_graph::NOT_CLAIMED.iter().map(|s| string(s)).collect();
    field(
        &mut out,
        "notClaimed",
        &format!("[{}]", not_claimed.join(",")),
    );
    out.push('}');
    out.push('\n');
    out
}

/// The eight grants and which of them a human asserts.
///
/// Printed whether or not the program uses any of them, because the point
/// of §19.5's completeness is that the set is closed: a reader comparing
/// `asserted` against this can see that the two entries with
/// `"asserted": true` are the only two there are.
fn closed_set() -> String {
    let entries: Vec<String> = Grant::CLOSED_LIST
        .iter()
        .map(|grant| {
            let mut out = String::from("{");
            field(&mut out, "code", &string(grant.code()));
            out.push(',');
            field(&mut out, "asserted", &grant.is_asserted().to_string());
            out.push('}');
            out
        })
        .collect();
    format!("[{}]", entries.join(","))
}

/// One `trusted p` clause — the other human signature, which E-REL-08's
/// help text already told the author would appear here.
fn endorsement(clause: &Endorsement, linked: &Linked) -> String {
    let mut out = String::from("{");
    field(&mut out, "release", &string(&printable(&clause.release)));
    out.push(',');
    field(
        &mut out,
        "parameter",
        &string(&printable(&clause.parameter)),
    );
    out.push(',');
    field(&mut out, "declaredAt", &place(clause.declared_at, linked));
    out.push('}');
    out
}

/// The prelude's grants, by name.
///
/// No spans, and [`zdc_graph::report::LIBRARY_NOTE`] — which is in
/// `notClaimed` above — says why: a prelude span indexes the library's own
/// file rather than anything the linked program holds, so locating one
/// would print a line number in the reader's file that means nothing.
fn library(grants: &zdc_graph::report::LibraryGrants) -> String {
    let names = |names: &[String]| {
        let quoted: Vec<String> = names.iter().map(|name| string(&printable(name))).collect();
        format!("[{}]", quoted.join(","))
    };
    let mut out = String::from("{");
    field(&mut out, "pure", &names(&grants.pure));
    out.push(',');
    field(&mut out, "trusted", &names(&grants.trusted));
    out.push(',');
    field(&mut out, "anywhere", &names(&grants.anywhere));
    out.push('}');
    out
}

fn asserted(grant: &AssertedGrant, linked: &Linked) -> String {
    let mut out = String::from("{");
    field(&mut out, "code", &string(grant.grant.code()));
    out.push(',');
    // The program's own identifiers reach this file, so they go through
    // `printable` for the reason every message does: a `.zd` identifier
    // cannot carry U+001B but a module specifier is a string literal and
    // can, and a report is `cat`ed by people as well as parsed.
    field(&mut out, "name", &string(&printable(&grant.name)));
    out.push(',');
    field(&mut out, "gives", &string(grant.marker));
    out.push(',');
    field(&mut out, "is", &string(grant.site));
    out.push(',');
    field(
        &mut out,
        "from",
        &match &grant.module {
            Some(module) => string(&printable(module)),
            None => "null".to_string(),
        },
    );
    out.push(',');
    field(&mut out, "export", &string(&printable(&grant.export)));
    out.push(',');
    field(&mut out, "primitive", &grant.primitive.to_string());
    out.push(',');
    field(&mut out, "declaredAt", &place(grant.declared_at, linked));
    out.push(',');
    let calls: Vec<String> = grant
        .calls
        .iter()
        .map(|span| place(*span, linked))
        .collect();
    field(&mut out, "calls", &format!("[{}]", calls.join(",")));
    out.push(',');
    let releases: Vec<String> = grant
        .releases
        .iter()
        .map(|reach| release(reach, linked))
        .collect();
    field(
        &mut out,
        "reachedByReleases",
        &format!("[{}]", releases.join(",")),
    );
    out.push('}');
    out
}

/// One `is anywhere` declaration.
///
/// No `reachedByReleases`, and the absence is the point rather than an
/// omission. That list answers *which declassification does this
/// assertion let compile*, which is a question about the integrity
/// lattice; a placement awards no authority and no `release` rests on
/// one. What a reviewer wants here is where it was declared and where it
/// is called, and claiming more would be the shape of thing §21.8.8
/// forbids.
fn anywhere(placement: &AnywherePlacement, linked: &Linked) -> String {
    let mut out = String::from("{");
    field(&mut out, "name", &string(&printable(&placement.name)));
    out.push(',');
    field(
        &mut out,
        "from",
        &match &placement.module {
            Some(module) => string(&printable(module)),
            None => "null".to_string(),
        },
    );
    out.push(',');
    field(&mut out, "export", &string(&printable(&placement.export)));
    out.push(',');
    field(&mut out, "primitive", &placement.primitive.to_string());
    out.push(',');
    field(&mut out, "declaredAt", &place(placement.declared_at, linked));
    out.push(',');
    let calls: Vec<String> = placement
        .calls
        .iter()
        .map(|span| place(*span, linked))
        .collect();
    field(&mut out, "calls", &format!("[{}]", calls.join(",")));
    out.push('}');
    out
}

fn release(reach: &ReleaseReach, linked: &Linked) -> String {
    let mut out = String::from("{");
    field(&mut out, "name", &string(&printable(&reach.name)));
    out.push(',');
    field(&mut out, "declaredAt", &place(reach.declared_at, linked));
    out.push(',');
    field(&mut out, "reachedAt", &place(reach.reached_at, linked));
    out.push('}');
    out
}

/// A span as a file and a place in it.
///
/// The same shape [`crate::json`] gives a diagnostic's span, and for the
/// same reason: a byte range is what the compiler carries, and a line and
/// column is what every editor and every reader wants. The `path` is here
/// rather than at the top of the document because a program is several
/// files — the prelude alone is eight — and a grant in one of them is not
/// in the entry file.
fn place(span: Span, linked: &Linked) -> String {
    let (path, source, local) = linked.locate(span);
    let (line, column) = position(source, local.start as usize);
    let mut out = String::from("{");
    field(
        &mut out,
        "file",
        &string(&printable(&path.display().to_string())),
    );
    out.push(',');
    field(&mut out, "line", &line.to_string());
    out.push(',');
    field(&mut out, "column", &column.to_string());
    out.push(',');
    field(&mut out, "start", &local.start.to_string());
    out.push(',');
    field(&mut out, "end", &local.end.to_string());
    out.push('}');
    out
}
