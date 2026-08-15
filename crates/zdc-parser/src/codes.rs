//! The rules a parse error can be an instance of.
//!
//! Every `ParseError` carries one of these, and `ParseError::code` is not
//! an `Option`, so a new error cannot be added without choosing. Until
//! this existed, `zdc explain` knew thirty codes, all from the placement,
//! information-flow, integrity and release passes: the errors with the
//! most careful prose were the ones a reader could ask about, and the ones
//! a beginner meets first were the ones they could not.
//!
//! **Why seven codes and not thirty-two.** A code names a *rule*, not a
//! call site. Thirty-two codes would put thirty-two entries behind
//! `zdc explain` saying the same three things in slightly different words,
//! and a reader who has met one of them would learn nothing from the next.
//! These seven are the rules the parser actually enforces; the message
//! still names the construct, because the message is where the specific
//! belongs.
//!
//! Each is scanned out of this file by `zdc-diagnostics`'s coverage gate,
//! which fails when a code here has no explanation and when an explanation
//! names a code nothing produces.

/// A `state` declaration did not say where its value lives.
///
/// The most common error in the language, because every `state`
/// declaration goes through the same function.
pub const PLACEMENT: &str = "E0101";

/// A keyword was written where a name goes.
pub const KEYWORD_AS_NAME: &str = "E0102";

/// The construct has one valid form (spec §4.1) and this is not it.
pub const ONE_VALID_FORM: &str = "E0103";

/// Nothing that can begin the construct this position expects.
pub const NO_SUCH_CONSTRUCT: &str = "E0104";

/// The source nests deeper than the compiler will follow.
pub const TOO_DEEP: &str = "E0105";

/// A route's URL is not a canonical absolute literal path.
pub const ROUTE_URL: &str = "E0106";

/// The program named a principal the language cannot establish.
///
/// Today that is exactly one construct, `durable per visitor`, and this
/// code exists so that construct gets a *refusal* rather than the parse
/// error it used to get. `per` is a soft keyword, so `durable per visitor
/// Whole` parsed `per` as the type and reported a missing `starting`
/// clause under a caret on `visitor` — telling a reader their type was
/// fine and their initialiser was absent, when neither was the problem.
///
/// **Why a refusal and not an implementation** (issue #17, issue #32's
/// N2). Per-principal storage needs a principal. The only channel a
/// principal could arrive on is the request, and `Host::invoke` takes an
/// endpoint name and a JSON argument array — there are no headers, no
/// cookies and no request context anywhere in `zdc-host`. Building one
/// means minting a session credential, which is authentication, which
/// §13 lists as a v1 non-goal in the same breath as per-user durable
/// scoping. The two are one non-goal because they are one problem.
///
/// **And the identity would not be a visitor.** An anonymous session
/// cookie is a bearer token naming a browser profile: a shared machine is
/// one partition for two people, one person on a phone and a laptop is
/// two partitions, and whoever holds the token is the principal. The
/// partition's secrecy would then rest on three things the compiler
/// cannot check — that the adapter sets `HttpOnly`, `Secure` and
/// `SameSite`; that the token comes from a CSPRNG; and that each of the
/// store backends honours the prefix rather than ignoring it. Those are
/// `zdc-graph`'s R5 shape, "asserted by a human and checked by nobody",
/// load-bearing this time for a *confidentiality* claim. A construct
/// spelled `per visitor` that delivers per-cookie separation on three
/// unchecked assumptions is the leak of issue #32 rebuilt with extra
/// steps, and it would read as isolation to everybody who used it.
///
/// So the parser refuses here, at the earliest point, and no
/// `DurablePerVisitor` reaches the AST. `SignalPlacement` keeps its
/// variant — it is the spec's table, and `zdc-types` classifies it
/// already — but nothing can construct one, which is the fail-closed
/// arrangement: an accidental acceptance downstream is impossible rather
/// than merely unlikely.
pub const NO_SUCH_PRINCIPAL: &str = "E0107";
