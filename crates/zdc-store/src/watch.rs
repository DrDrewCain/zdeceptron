//! Fan-out: the genuinely hard half of live sync (§8.1).
//!
//! §8.1 corrects an earlier claim in the design and is worth restating,
//! because it decides what lives here. Holding a stream open is solved on
//! every platform — Lambda response streaming, Cloudflare hibernatable
//! sockets, Vercel fluid compute, or an ordinary open response on a
//! container. **What is hard is telling client B that client A wrote.**
//! `DurableStore::watch` is the seam where each deployment target plugs in
//! its own primitive: a Durable Object, DynamoDB Streams, Upstash pub/sub
//! — or, here, an in-process broadcast beside the local database, which is
//! the row §8.1's table calls "local dev".
//!
//! The other half of §8.1 is the reconnect discipline, and it is the
//! reason [`Seq`] exists. SSE reconnects by itself and resumes from
//! `Last-Event-ID`; a resume that cannot prove it has every update in
//! between must say so rather than continue silently, so a subscription
//! either replays the exact tail a client is missing or opens with
//! [`Event::Resync`]. There is no third answer, which is what makes
//! "never drops and never duplicates" checkable.

use std::collections::VecDeque;
use std::sync::mpsc::{self, TryRecvError};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use crate::value::Json;

/// Where a write sits in the store's single total order.
///
/// One counter for the whole store rather than one per key. A per-key
/// counter would make a client's "I have seen everything up to here" claim
/// a vector rather than a number, and `Last-Event-ID` is a single header
/// carrying a single string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Seq(pub u64);

impl Seq {
    pub fn next(self) -> Seq {
        Seq(self.0 + 1)
    }
}

impl std::fmt::Display for Seq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One write, as a subscriber sees it.
///
/// `value` is `None` for a delete. Sending the value rather than only the
/// key is §17.2.5 fatal 4's `LiveValue` edge — "the browser is sent the
/// VALUE" — and it is what lets a second window update without a round
/// trip. Whether that is *allowed* for a given key is not decided here:
/// the split records the edge and the information-flow pass rules on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub seq: Seq,
    pub key: String,
    pub value: Option<Json>,
}

/// What a subscriber is handed, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The tail this subscriber missed is no longer retained, so the
    /// updates in between cannot be replayed. Re-read what you care
    /// about; `seq` is the position that re-read is current as of.
    ///
    /// Emitted rather than skipped on purpose. Continuing silently is
    /// exactly the dropped update §8.1 forbids, and it is invisible in
    /// testing because it only happens after a real disconnection.
    Resync {
        seq: Seq,
    },
    Update(Update),
}

/// How many updates are kept for replaying to a client that reconnects.
///
/// A reconnect on a dropped SSE stream takes the `retry:` interval, so the
/// window a subscriber can be away for is well under a second and this is
/// generous. It is bounded rather than unbounded because an unbounded
/// backlog is a memory leak keyed on how long the process has run.
const BACKLOG: usize = 256;

#[derive(Default)]
struct Inner {
    /// `(prefix, sender)`. A dead sender is reaped on the next publish,
    /// which is the same way the dev server reaps closed browser tabs.
    subscribers: Vec<(String, Sender<Update>)>,
    backlog: VecDeque<Update>,
    latest: Seq,
}

/// The broadcast beside the database.
#[derive(Default)]
pub struct Fanout {
    inner: Mutex<Inner>,
}

impl Fanout {
    pub fn new(latest: Seq) -> Fanout {
        Fanout {
            inner: Mutex::new(Inner {
                subscribers: Vec::new(),
                backlog: VecDeque::new(),
                latest,
            }),
        }
    }

    /// Announce a committed write.
    ///
    /// Called after the transaction commits, never before: a subscriber
    /// that was told about a write which then failed to commit would show
    /// a value no reader can read.
    pub fn publish(&self, update: Update) {
        let mut inner = self.inner.lock().expect("durable store fanout poisoned");
        inner.latest = update.seq;
        if inner.backlog.len() == BACKLOG {
            inner.backlog.pop_front();
        }
        inner.backlog.push_back(update.clone());
        inner.subscribers.retain(|(prefix, tx)| {
            !update.key.starts_with(prefix.as_str()) || tx.send(update.clone()).is_ok()
        });
    }

    pub fn latest(&self) -> Seq {
        self.inner
            .lock()
            .expect("durable store fanout poisoned")
            .latest
    }

    /// Subscribe to every key under `prefix`, resuming after `since`.
    ///
    /// Registration and the backlog snapshot happen under one lock, so a
    /// write landing mid-subscribe is either in the replayed tail or on
    /// the channel — never in neither, which is the only way it could be
    /// lost.
    pub fn subscribe(&self, prefix: &str, since: Option<Seq>) -> Subscription {
        let (tx, rx) = mpsc::channel();
        let mut inner = self.inner.lock().expect("durable store fanout poisoned");
        inner.subscribers.push((prefix.to_string(), tx));

        let latest = inner.latest;
        let pending = match since {
            // A fresh stream. The client has just loaded the page and its
            // `$remote` cells fetch on their own, so it is current by
            // construction — the same reading `zdc-dev`'s live-reload
            // stream already gives an absent `Last-Event-ID`.
            None => VecDeque::new(),
            Some(seen) if seen >= latest => VecDeque::new(),
            Some(seen) => match inner.backlog.front() {
                // The retained tail reaches back far enough to cover
                // everything after `seen`, because sequence numbers are
                // consecutive.
                Some(oldest) if oldest.seq.0 <= seen.0 + 1 => inner
                    .backlog
                    .iter()
                    .filter(|update| update.seq > seen && update.key.starts_with(prefix))
                    .cloned()
                    .map(Event::Update)
                    .collect(),
                Some(_) => VecDeque::from([Event::Resync { seq: latest }]),
                None => VecDeque::from([Event::Resync { seq: latest }]),
            },
        };

        Subscription {
            pending,
            rx,
            seq: latest,
        }
    }

    /// How many subscribers are attached. The dev server reports this, and
    /// a test that asserts a window disconnected has nothing else to look
    /// at.
    pub fn subscribers(&self) -> usize {
        self.inner
            .lock()
            .expect("durable store fanout poisoned")
            .subscribers
            .len()
    }
}

/// One client's view of the stream.
pub struct Subscription {
    pending: VecDeque<Event>,
    rx: Receiver<Update>,
    seq: Seq,
}

impl Subscription {
    /// The position the stream opened at — the id a `ready` frame carries,
    /// so a client that sees no traffic still learns where it is.
    pub fn seq(&self) -> Seq {
        self.seq
    }

    /// The next event, or `None` if none is ready yet.
    pub fn try_next(&mut self) -> Option<Event> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        match self.rx.try_recv() {
            Ok(update) => Some(Event::Update(update)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// The next event, waiting up to `timeout`.
    ///
    /// `None` on timeout is not an error: it is the cue to write a
    /// heartbeat comment down the stream, which is how the server finds
    /// out that a tab was closed.
    pub fn next_timeout(&mut self, timeout: Duration) -> Option<Event> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        match self.rx.recv_timeout(timeout) {
            Ok(update) => Some(Event::Update(update)),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(seq: u64, key: &str, value: &str) -> Update {
        Update {
            seq: Seq(seq),
            key: key.to_string(),
            value: Some(Json::from_text(value)),
        }
    }

    #[test]
    fn a_subscriber_hears_a_write_that_lands_after_it_subscribed() {
        let fanout = Fanout::default();
        let mut window = fanout.subscribe("", None);
        fanout.publish(update(1, "visits", "1"));
        assert_eq!(
            window.try_next(),
            Some(Event::Update(update(1, "visits", "1")))
        );
    }

    #[test]
    fn two_subscribers_both_hear_one_write() {
        // The two-window demo, reduced to the fan-out it depends on.
        let fanout = Fanout::default();
        let mut a = fanout.subscribe("", None);
        let mut b = fanout.subscribe("", None);
        fanout.publish(update(1, "visits", "1"));
        assert!(a.try_next().is_some());
        assert!(b.try_next().is_some());
    }

    #[test]
    fn a_subscriber_hears_nothing_outside_its_prefix() {
        let fanout = Fanout::default();
        let mut window = fanout.subscribe("session/7/", None);
        fanout.publish(update(1, "visits", "1"));
        assert_eq!(window.try_next(), None);
        fanout.publish(update(2, "session/7/cart", "[]"));
        assert!(window.try_next().is_some());
    }

    #[test]
    fn a_fresh_subscriber_is_told_nothing_it_already_has() {
        let fanout = Fanout::default();
        fanout.publish(update(1, "visits", "1"));
        let mut window = fanout.subscribe("", None);
        assert_eq!(window.try_next(), None, "a fresh stream replays nothing");
        assert_eq!(window.seq(), Seq(1));
    }

    #[test]
    fn a_reconnecting_subscriber_is_replayed_exactly_what_it_missed() {
        // The `Last-Event-ID` case §8.1 requires: no drop and no duplicate.
        let fanout = Fanout::default();
        fanout.publish(update(1, "visits", "1"));
        fanout.publish(update(2, "visits", "2"));
        fanout.publish(update(3, "visits", "3"));

        let mut window = fanout.subscribe("", Some(Seq(1)));
        assert_eq!(
            window.try_next(),
            Some(Event::Update(update(2, "visits", "2")))
        );
        assert_eq!(
            window.try_next(),
            Some(Event::Update(update(3, "visits", "3")))
        );
        assert_eq!(window.try_next(), None, "nothing is delivered twice");
    }

    #[test]
    fn a_subscriber_that_is_already_current_is_replayed_nothing() {
        let fanout = Fanout::default();
        fanout.publish(update(1, "visits", "1"));
        let mut window = fanout.subscribe("", Some(Seq(1)));
        assert_eq!(window.try_next(), None);
    }

    #[test]
    fn a_gap_the_backlog_cannot_cover_is_announced_rather_than_skipped() {
        // Silently continuing here is the dropped update §8.1 forbids, and
        // it is invisible until a real disconnection produces it.
        let fanout = Fanout::new(Seq(0));
        for n in 1..=(BACKLOG as u64 + 5) {
            fanout.publish(update(n, "visits", &n.to_string()));
        }
        let mut window = fanout.subscribe("", Some(Seq(1)));
        assert_eq!(
            window.try_next(),
            Some(Event::Resync {
                seq: Seq(BACKLOG as u64 + 5)
            })
        );
    }

    #[test]
    fn a_restarted_store_with_an_empty_backlog_announces_a_gap() {
        // The seq counter survives a restart because it is persisted; the
        // backlog does not, so an open tab reconnecting across a restart
        // must re-read rather than assume nothing happened.
        let fanout = Fanout::new(Seq(9));
        let mut window = fanout.subscribe("", Some(Seq(4)));
        assert_eq!(window.try_next(), Some(Event::Resync { seq: Seq(9) }));
    }

    #[test]
    fn a_dropped_subscriber_is_reaped_on_the_next_write() {
        let fanout = Fanout::default();
        let window = fanout.subscribe("", None);
        assert_eq!(fanout.subscribers(), 1);
        drop(window);
        fanout.publish(update(1, "visits", "1"));
        assert_eq!(fanout.subscribers(), 0);
    }
}
