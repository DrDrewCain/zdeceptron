//! Build-time refusals.
//!
//! The rest of the compiler refuses to emit a program whose placements do
//! not resolve, rather than emitting something that fails at run time. A
//! deploy target is the same kind of claim: a program whose live sync
//! cannot work on the platform it is aimed at should not build, and the
//! message should name the limitation rather than the rule.

/// A combination of program and platform that cannot work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub message: String,
}

impl Refusal {
    pub(crate) fn new(message: impl Into<String>) -> Refusal {
        Refusal {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Refusal {}
