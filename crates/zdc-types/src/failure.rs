//! `code`: the one field of a `Failed` payload the client runtime writes
//! from its own control flow.
//!
//! §14G.1.3(d) puts the join of everything the endpoint read onto the
//! `Failed` payload, and it is right to: an HTTP client's error text
//! routinely quotes the request it was making, key and all. The
//! consequence is that `message` is unreadable from exactly the endpoints
//! a developer most wants explained.
//!
//! `code` is the other field. It is not derived from the response at all.
//! `runtime/rpc.js` picks it from the transport outcome — whether an HTTP
//! response arrived, whether the client's own deadline fired first, and
//! the status line if one did arrive — and never from the response body.
//! Its provenance is the runtime's own control flow, so no byte a server
//! chooses to write selects it, and it carries `public` rather than the
//! join. §17.6 item 15 makes records field-insensitive; this is the one
//! exception, and `zdc-graph`'s flow pass admits it only for a binder a
//! `Failed` pattern introduced.
//!
//! **The set is three, not four.** A fourth candidate, `Malformed` — "the
//! body did not parse" — was specified and is not here. It is selected by
//! the response *body*, so a server that could vary well-formedness on a
//! secret bit would have one bit at a public label, which is the channel
//! §14G.1.3(b) exists to close. A 2xx whose body does not decode is
//! reported as [`FailureCode::Rejected`] instead, which the status line
//! can already produce on its own — so the body distinguishes nothing
//! that the status line does not.

/// What the client runtime observed instead of an answer.
///
/// Closed, and the workspace's wildcard-arm check guards it (see
/// `scripts/check-wildcard-arms.sh`): a fourth variant is a compile error
/// at every site that has to consider one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureCode {
    /// No HTTP response arrived: the connection failed, was refused, or
    /// was cut. A server can reach this by declining to answer, which is
    /// not a byte it sends.
    Unreachable,
    /// The client's own deadline elapsed first. Chosen by the browser's
    /// clock, not by anything on the wire. A slow server can reach it,
    /// which is the timing channel §17.7 already records as unmodelled.
    Timeout,
    /// A response arrived and was not usable: a non-2xx status line, or a
    /// 2xx whose body the wire decoder could not read.
    Rejected,
}

impl FailureCode {
    /// Every code, in the order a diagnostic lists them.
    ///
    /// Deliberately not accompanied by an assertion on its length: a
    /// `[FailureCode; N]` compared against `N` compares a constant to
    /// itself. What holds the set closed is [`FailureCode::spelling`]
    /// having no wildcard arm, and the check that forbids one.
    pub const CLOSED_SET: [FailureCode; 3] = [
        FailureCode::Unreachable,
        FailureCode::Timeout,
        FailureCode::Rejected,
    ];

    /// How the code reads in a program: the exact `Text` the runtime puts
    /// in the field, so `error.code is "Timeout"` means what it looks
    /// like.
    pub fn spelling(self) -> &'static str {
        match self {
            FailureCode::Unreachable => "Unreachable",
            FailureCode::Timeout => "Timeout",
            FailureCode::Rejected => "Rejected",
        }
    }

    /// What the runtime saw, for a hover and for the explanation of the
    /// rule.
    pub fn observed(self) -> &'static str {
        match self {
            FailureCode::Unreachable => "no response arrived",
            FailureCode::Timeout => "the client's deadline elapsed first",
            FailureCode::Rejected => "a response arrived and was not usable",
        }
    }

    /// Where this code sits in [`FailureCode::CLOSED_SET`].
    ///
    /// It exists so that the set cannot silently omit a variant. The two
    /// `match`es above already make a fourth variant a compile error, but
    /// a `[FailureCode; 3]` would happily go on listing three of four —
    /// and everything that iterates the set would quietly skip the new
    /// one. This match is the third compile error, and the test below
    /// turns a wrong answer into a failure rather than a silent overlap.
    pub fn position(self) -> usize {
        match self {
            FailureCode::Unreachable => 0,
            FailureCode::Timeout => 1,
            FailureCode::Rejected => 2,
        }
    }

    /// The code a spelling denotes, for a test or an editor that has the
    /// text and wants the variant.
    pub fn from_spelling(text: &str) -> Option<FailureCode> {
        FailureCode::CLOSED_SET
            .into_iter()
            .find(|code| code.spelling() == text)
    }
}

/// The spellings, quoted and joined, for a diagnostic that lists them.
pub fn code_spellings() -> String {
    let quoted: Vec<String> = FailureCode::CLOSED_SET
        .iter()
        .map(|code| format!("`\"{}\"`", code.spelling()))
        .collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{}, and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spellings are distinct and each round-trips. Not a length
    /// assertion: it names the three codes in the test's own text, so
    /// dropping one from the enum stops this compiling and adding one
    /// leaves it unmentioned here and in `from_spelling`'s domain.
    #[test]
    fn every_code_round_trips_through_its_spelling() {
        assert_eq!(
            FailureCode::from_spelling("Unreachable"),
            Some(FailureCode::Unreachable)
        );
        assert_eq!(
            FailureCode::from_spelling("Timeout"),
            Some(FailureCode::Timeout)
        );
        assert_eq!(
            FailureCode::from_spelling("Rejected"),
            Some(FailureCode::Rejected)
        );
    }

    /// A spelling the set does not contain is not a code. `Malformed` is
    /// the one that was specified and dropped, and it is named here so
    /// that re-adding it has to come past this test.
    #[test]
    fn a_body_derived_code_is_not_in_the_set() {
        assert_eq!(FailureCode::from_spelling("Malformed"), None);
        assert_eq!(FailureCode::from_spelling("timeout"), None);
        assert_eq!(FailureCode::from_spelling(""), None);
    }

    #[test]
    fn the_set_lists_itself_for_a_diagnostic() {
        assert_eq!(
            code_spellings(),
            "`\"Unreachable\"`, `\"Timeout\"`, and `\"Rejected\"`"
        );
    }

    /// Every variant is in the set, at its own place.
    ///
    /// `position` is an exhaustive match, so a fourth variant stops this
    /// file compiling; this test is what stops the fourth variant being
    /// added to `position` with an index the set does not have, or with
    /// one another variant already occupies.
    #[test]
    fn the_closed_set_holds_every_code_once() {
        let mut placed = 0;
        for code in FailureCode::CLOSED_SET {
            assert_eq!(
                FailureCode::CLOSED_SET.get(code.position()),
                Some(&code),
                "{code:?} is not in the set at the position it claims"
            );
            placed += 1;
        }
        assert_eq!(
            placed,
            FailureCode::CLOSED_SET.len(),
            "the loop skipped a code"
        );
    }

    /// Two codes never share a spelling, so reading one back is
    /// unambiguous. Counted, so an empty or truncated set fails here
    /// rather than passing over nothing.
    #[test]
    fn the_spellings_are_distinct() {
        let mut seen: Vec<&str> = FailureCode::CLOSED_SET
            .iter()
            .map(|code| code.spelling())
            .collect();
        let scanned = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), scanned);
        assert_eq!(seen, ["Rejected", "Timeout", "Unreachable"]);
    }
}
