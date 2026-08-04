//! Server-Sent Events framing.
//!
//! Live reload is the first user, but not the last: spec §8.1 makes SSE the
//! transport for `durable` cross-client sync, and requires that the runtime
//! "send event IDs and resume from `Last-Event-ID` so a reconnect never
//! drops or duplicates an update". The id and resume rules below are that
//! requirement, implemented once here rather than twice later.
//!
//! Framing is kept as free functions over strings so it can be tested
//! without a socket, a browser, or a running server.

/// Where the dev server publishes its event stream.
pub const LIVE_PATH: &str = "/__zdc/live";

/// Sent once per connection so a client that reconnected after the server
/// restarted can tell whether it missed anything.
pub const READY: &str = "ready";

/// Sent when the served bundle changed and the page should be re-fetched.
pub const RELOAD: &str = "reload";

/// Every event name this module frames, with one home.
///
/// `tests/live_client.rs` asserts that the injected script registers a
/// handler for each of these by *running* the script. It used to compare
/// against the literal `"ready,reload"`, which meant a third event could
/// be added and the client left deaf to it with nothing failing. A new
/// event belongs here, and the day it is added that test fails until the
/// client learns to listen for it.
pub const EVENTS: [&str; 2] = [READY, RELOAD];

/// How long a client waits before reconnecting, in milliseconds.
///
/// Deliberately short. The gap between a dropped connection and the
/// reconnect is a window in which an edit produces no reload, and on a
/// loopback connection there is nothing to back off from.
pub const RETRY_MS: u64 = 500;

/// The preamble every stream opens with: the reconnection delay, as a
/// field on its own so it survives even if the first event is far off.
pub fn preamble() -> String {
    format!("retry: {RETRY_MS}\n\n")
}

/// One event.
///
/// `data` is split on newlines because a bare `\n` inside a `data:` field
/// would terminate the event: a multi-line payload is many `data:` lines
/// which the client rejoins with `\n`. Compile diagnostics are multi-line,
/// so this is not hypothetical.
pub fn frame(id: u64, event: &str, data: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("id: {id}\n"));
    out.push_str(&format!("event: {event}\n"));
    for line in data.split('\n') {
        out.push_str("data: ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    out
}

/// A comment, which carries no event but keeps the connection warm.
///
/// Without traffic an idle stream can be closed by an intermediary, and —
/// more importantly here — the server never learns that the browser tab
/// was closed, because a socket with nothing written to it never fails.
pub fn comment(text: &str) -> String {
    format!(": {text}\n\n")
}

/// What a newly connected client is owed, given the id it last saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// The client is current: send `ready` and then wait.
    UpToDate,
    /// The client missed at least one rebuild — it was disconnected while
    /// the source changed, or the server restarted under it. Reload now
    /// rather than leaving a stale page on screen until the next edit.
    Missed,
}

/// Decide what a reconnecting client needs from its `Last-Event-ID`.
///
/// An absent or unparseable header means a fresh `EventSource`, which has
/// just loaded the current page and is by definition up to date. Only a
/// header that is genuinely behind the current generation earns a reload.
pub fn resume(last_event_id: Option<&str>, generation: u64) -> Resume {
    match last_event_id.map(str::trim).map(str::parse::<u64>) {
        Some(Ok(seen)) if seen < generation => Resume::Missed,
        _ => Resume::UpToDate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_carries_an_id_an_event_and_a_data_line() {
        assert_eq!(frame(7, RELOAD, "7"), "id: 7\nevent: reload\ndata: 7\n\n");
    }

    #[test]
    fn a_frame_ends_with_a_blank_line_so_the_client_dispatches_it() {
        // An event the client never dispatches is an event that never
        // happened, and the only thing that dispatches one is the blank
        // line. Asserted separately because it is the easiest byte to lose.
        let f = frame(1, READY, "1");
        assert!(f.ends_with("\n\n"), "no dispatching blank line in {f:?}");
    }

    #[test]
    fn multi_line_data_becomes_one_data_field_per_line() {
        // A raw newline inside a `data:` field would end the event early
        // and the client would see a truncated payload.
        let f = frame(2, "diagnostic", "line one\nline two");
        assert_eq!(
            f,
            "id: 2\nevent: diagnostic\ndata: line one\ndata: line two\n\n"
        );
    }

    #[test]
    fn empty_data_still_produces_a_data_field() {
        assert_eq!(frame(0, READY, ""), "id: 0\nevent: ready\ndata: \n\n");
    }

    #[test]
    fn the_preamble_sets_the_reconnection_delay() {
        assert_eq!(preamble(), "retry: 500\n\n");
    }

    #[test]
    fn a_comment_is_a_colon_line_and_dispatches_nothing() {
        let c = comment("keep-alive");
        assert!(c.starts_with(':'), "not a comment: {c:?}");
        assert!(
            !c.contains("data:"),
            "a heartbeat must carry no data: {c:?}"
        );
        assert_eq!(c, ": keep-alive\n\n");
    }

    #[test]
    fn a_fresh_client_without_a_last_event_id_is_up_to_date() {
        assert_eq!(resume(None, 9), Resume::UpToDate);
    }

    #[test]
    fn a_client_that_saw_the_current_generation_is_up_to_date() {
        assert_eq!(resume(Some("9"), 9), Resume::UpToDate);
    }

    #[test]
    fn a_client_behind_the_current_generation_missed_a_rebuild() {
        assert_eq!(resume(Some("8"), 9), Resume::Missed);
    }

    #[test]
    fn a_last_event_id_that_is_not_a_number_is_not_treated_as_behind() {
        // The header is attacker-adjacent only in the sense that it comes
        // from outside; either way, garbage must not be read as "missed"
        // and put the page into a reload loop.
        assert_eq!(resume(Some("nonsense"), 9), Resume::UpToDate);
        assert_eq!(resume(Some(""), 9), Resume::UpToDate);
        assert_eq!(resume(Some("-1"), 9), Resume::UpToDate);
    }

    #[test]
    fn a_last_event_id_is_trimmed_before_it_is_parsed() {
        // Header values arrive with the leading space from `Name: value`
        // already stripped by most parsers, but not by all of them.
        assert_eq!(resume(Some(" 8 "), 9), Resume::Missed);
    }
}
