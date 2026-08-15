//! The `/_zd/` surface: what the browser posts to and subscribes to.
//!
//! One module because the three paths are one contract, and the client
//! half of it is in `runtime/store.js` and `runtime/rpc.js`. Splitting the
//! URL shapes across the server file and the transport file is how the two
//! come to disagree about where the cursor rides.
//!
//! ```text
//! POST /_zd/<endpoint>          run a server function, JSON in, JSON out
//! GET  /_zd/live?keys=&since=   hold a stream open  (Last-Event-ID resumes)
//! GET  /_zd/poll?keys=&since=   ask once            (?since= resumes)
//! ```
//!
//! The last two carry the *same events*. That is what makes the transport
//! a seam: a target that cannot hold a stream — Lambda in buffered mode,
//! Lambda behind an ALB — polls, and nothing above the transport changes.

use zdc_store::{Event, Seq};

/// The prefix every generated call goes under.
pub const PREFIX: &str = "/_zd/";

/// Where a held stream lives.
pub const LIVE: &str = "/_zd/live";

/// Where the polling fallback lives.
pub const POLL: &str = "/_zd/poll";

/// Where one handler's whole write set is posted.
///
/// The body is `[[endpoint, args], ...]` in source order, and the server
/// commits every one of them or none. `~` cannot appear in a ZD
/// identifier, so this can never collide with an endpoint a program
/// declares.
pub const ATOMIC: &str = "/_zd/~atomic";

/// The `event:` name a durable write is announced under.
///
/// Distinct from the live-reload channel's names so one `EventSource` can
/// carry both: a browser is limited to six connections per origin, and
/// spending two of them on one page's two concerns is a cost with nothing
/// bought for it.
pub const UPDATE: &str = "update";

/// The endpoint name a request path addresses, if it addresses one.
///
/// `live` and `poll` are excluded because they are the transport, not
/// endpoints — a program with a `state live` would otherwise have its read
/// endpoint shadowed by the subscription URL.
pub fn invocation(path: &str) -> Option<String> {
    let name = path.strip_prefix(PREFIX)?;
    if name.is_empty() || path == LIVE || path == POLL || path == ATOMIC {
        return None;
    }
    Some(decode(name))
}

/// Percent-decoding, for the `encodeURIComponent` the client applies.
///
/// An endpoint name is a program identifier plus a `.` and a verb, so in
/// practice nothing needs decoding — which is exactly why the one case
/// that does would go unnoticed without this.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The query a subscription carries: which keys, and from where.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    keys: Vec<String>,
    since: Option<u64>,
    /// The wire format the subscriber named (#144), verbatim.
    ///
    /// Kept as text rather than parsed to a number, because the check is
    /// an equality against a spelling and "absent" and "unparseable" must
    /// not be collapsed into the same `None` as "named something else" —
    /// the sentence a developer reads quotes what arrived.
    wire: Option<String>,
}

impl Query {
    pub fn of(url: &str) -> Query {
        let Some((_, query)) = url.split_once('?') else {
            return Query::default();
        };
        let mut parsed = Query::default();
        for pair in query.split('&') {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            if name == "keys" {
                parsed.keys = decode(value)
                    .split(',')
                    .filter(|key| !key.is_empty())
                    .map(str::to_string)
                    .collect();
            } else if name == "since" {
                parsed.since = decode(value).parse().ok();
            } else if name == zdc_runtime::WIRE_VERSION_PARAM {
                parsed.wire = Some(decode(value));
            }
        }
        parsed
    }

    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    /// The wire format version the subscriber named, if it named one.
    pub fn wire(&self) -> Option<&str> {
        self.wire.as_deref()
    }

    /// Where to resume from.
    ///
    /// `None` for an absent or unparseable cursor, which is a fresh
    /// subscriber: its cells fetch on their own, so it is current by
    /// construction. Reading garbage as position 0 would replay the whole
    /// retained backlog to a client that needs none of it.
    pub fn since(&self) -> Option<Seq> {
        self.since.map(Seq)
    }
}

/// One event, as JSON, in the shape `runtime/store.js` decodes.
///
/// A delete carries `null`. `undefined` is not JSON, and the client's
/// `Ready(null)` is the right rendering of "this key is gone" — the
/// alternative, omitting the field, would be indistinguishable from a
/// value the server forgot to send.
pub fn payload(event: &Event) -> String {
    match event {
        Event::Resync { seq } => format!("{{\"event\":\"resync\",\"seq\":{}}}", seq.0),
        Event::Update(update) => format!(
            "{{\"event\":\"update\",\"seq\":{},\"key\":{},\"value\":{}}}",
            update.seq.0,
            json_string(&update.key),
            update
                .value
                .as_ref()
                .map(|json| json.as_str())
                .unwrap_or("null"),
        ),
    }
}

/// The sequence number an event should be sent under, which is the id a
/// reconnecting client sends back.
pub fn position(event: &Event) -> Seq {
    match event {
        Event::Resync { seq } => *seq,
        Event::Update(update) => update.seq,
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if c == '"' {
            out.push_str("\\\"");
        } else if c == '\\' {
            out.push_str("\\\\");
        } else if (c as u32) < 0x20 {
            out.push_str(&format!("\\u{:04x}", c as u32));
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zdc_store::{Json, Update};

    #[test]
    fn a_generated_call_is_recognised_by_its_prefix() {
        assert_eq!(invocation("/_zd/greeting"), Some("greeting".to_string()));
        assert_eq!(
            invocation("/_zd/visits.incr"),
            Some("visits.incr".to_string())
        );
    }

    #[test]
    fn the_only_transport_spelling_is_the_one_the_client_asks_for() {
        // `~watch` was a second name for `live`, generated by the deploy
        // adapters. One spelling, and it is the one `runtime/store.js`
        // emits, so an old adapter's URL is a 404 rather than a silent
        // second protocol.
        assert_eq!(LIVE, "/_zd/live");
        assert_eq!(POLL, "/_zd/poll");
        assert_eq!(
            invocation("/_zd/~watch"),
            Some("~watch".to_string()),
            "the retired alias is no longer a transport path"
        );
    }

    #[test]
    fn the_transport_paths_are_not_endpoints() {
        // A program with `state live` would otherwise have its read
        // endpoint shadowed by the subscription URL, and the failure would
        // look like a hanging page rather than a name collision.
        assert_eq!(invocation("/_zd/live"), None);
        // `~watch` is deliberately absent: it was retired as a transport
        // spelling, and `the_only_transport_spelling_is_the_one_the_client_asks_for`
        // is where that is now asserted.
        // The transaction endpoint names its writes in the body, not in
        // the path. Reading it as an endpoint name would look up an
        // endpoint called `~atomic` and 404 every multi-write handler.
        assert_eq!(invocation("/_zd/~atomic"), None);
        assert_eq!(invocation("/_zd/poll"), None);
        assert_eq!(invocation("/_zd/"), None);
        assert_eq!(invocation("/client.js"), None);
    }

    #[test]
    fn an_escaped_endpoint_name_is_decoded() {
        assert_eq!(invocation("/_zd/a%2Fb"), Some("a/b".to_string()));
    }

    #[test]
    fn a_subscription_query_carries_keys_and_a_cursor() {
        let query = Query::of("/_zd/live?keys=visits%2Cvotes&since=7");
        assert_eq!(query.keys(), ["visits".to_string(), "votes".to_string()]);
        assert_eq!(query.since(), Some(Seq(7)));
    }

    #[test]
    fn a_missing_or_unusable_cursor_is_a_fresh_subscriber() {
        // Not position 0. A fresh subscriber's cells fetch on their own, so
        // replaying the whole retained backlog to it would be pure noise —
        // and on a reconnect it would be duplicates.
        assert_eq!(Query::of("/_zd/live?keys=visits").since(), None);
        assert_eq!(Query::of("/_zd/live?keys=visits&since=").since(), None);
        assert_eq!(Query::of("/_zd/live?keys=visits&since=nope").since(), None);
        assert_eq!(Query::of("/_zd/live").keys(), Vec::<String>::new());
    }

    #[test]
    fn an_update_carries_the_value_and_not_only_the_key() {
        // §17.2.5 fatal 4's `LiveValue` edge. Sending only the key would
        // make every announcement cost the second window a round trip.
        let event = Event::Update(Update {
            seq: Seq(3),
            key: "visits".to_string(),
            value: Some(Json::from_text("7")),
        });
        assert_eq!(
            payload(&event),
            "{\"event\":\"update\",\"seq\":3,\"key\":\"visits\",\"value\":7}"
        );
        assert_eq!(position(&event), Seq(3));
    }

    #[test]
    fn a_delete_is_announced_as_a_null_value() {
        let event = Event::Update(Update {
            seq: Seq(4),
            key: "visits".to_string(),
            value: None,
        });
        assert_eq!(
            payload(&event),
            "{\"event\":\"update\",\"seq\":4,\"key\":\"visits\",\"value\":null}"
        );
    }

    #[test]
    fn a_resync_says_where_a_re_read_would_be_current_as_of() {
        let event = Event::Resync { seq: Seq(9) };
        assert_eq!(payload(&event), "{\"event\":\"resync\",\"seq\":9}");
        assert_eq!(position(&event), Seq(9));
    }
}
