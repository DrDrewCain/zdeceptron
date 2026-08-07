//! The rules a parse error can be an instance of.
//!
//! Every `ParseError` carries one of these, and `ParseError::code` is not
//! an `Option`, so a new error cannot be added without choosing. Until
//! this existed, `zdc explain` knew thirty codes, all from the placement,
//! information-flow, integrity and release passes: the errors with the
//! most careful prose were the ones a reader could ask about, and the ones
//! a beginner meets first were the ones they could not.
//!
//! **Why six codes and not thirty-one.** A code names a *rule*, not a call
//! site. Thirty-one codes would put thirty-one entries behind `zdc explain`
//! saying the same three things in slightly different words, and a reader
//! who has met one of them would learn nothing from the next. These six
//! are the rules the parser actually enforces; the message still names the
//! construct, because the message is where the specific belongs.
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
