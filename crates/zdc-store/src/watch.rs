//! Fan-out: the genuinely hard half of live sync (§8.1).
//!
//! # §8.1 is wrong about the easy half, and the research says so
//!
//! §8.1 claims holding a stream open is solved everywhere. The runtime
//! research of 2026-08-02 checked that against vendor documentation and it
//! is false in two common shapes: **Lambda in buffered mode cannot stream
//! at all** (the response is delivered only when complete), and **Lambda
//! behind an ALB cannot either** (the ALB takes a 1 MB JSON body, rejects
//! upgrades, and does not honour `Transfer-Encoding`). Where streaming does
//! work the ceiling spans three orders of magnitude — unbounded on
//! Cloudflare Workers, 900 s on a Lambda function URL, 300–800 s on Vercel,
//! a contested 230 s on Azure — and Deno Deploy may evict an isolate
//! mid-stream at any moment.
//!
//! Two consequences are baked into this module rather than left to an
//! adapter. **The transport is a seam, not a choice**: everything here is
//! expressed as "a cursor goes in, ordered events come out", which a held
//! stream and a poll loop implement identically. And **resume is
//! load-bearing rather than polish**: on Lambda the stream *will* be cut at
//! 900 s, so [`Seq`] and the replay below are what make a bounded stream
//! behave like an unbounded one.
//!
//! # Why this watches keys and not a prefix
//!
//! §7.4 spells the fifth operation `watch(prefix)`. That interface cannot
//! be honoured by the targets it exists for:
//!
//! - **Deno KV** is the only store in the survey with a literal `watch()`,
//!   and it takes an **explicit key list — there is no prefix variant**.
//! - **DynamoDB Streams** are pull-based change data capture with a hard
//!   cap of two readers per shard, not a subscribe-by-pattern channel.
//! - **Cloudflare KV** has no watch and allows one write per second per
//!   key, so it cannot back `incr` at all, let alone a change feed.
//! - **Durable Objects** push natively, but by being the addressable hub
//!   for a *named topic*; the storage API has no watch to filter.
//! - **Upstash** — the only real store on Vercel — has `SUBSCRIBE`, which
//!   takes channels.
//!
//! An interface only the local implementation can satisfy is not an
//! interface. So [`Fanout::subscribe`] takes **the set of keys the caller
//! wants**, which every one of those primitives can serve: it is Deno KV's
//! signature exactly, one channel per key on Upstash and AppSync, and one
//! topic per key on a Durable Object.
//!
//! Nothing is lost by it. A ZDeceptron program's durable keys are **fixed
//! at compile time** — they are `state ... is durable` declarations, and
//! `manifest.json` already lists them — so a client never needs to ask for
//! keys that might appear later. A prefix would only matter for the
//! per-visitor scoping §5.7 defers past v1, and when that arrives the
//! session's keys are still enumerable by the compiler.

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

/// Which keys a subscriber wants.
///
/// A set rather than a prefix, for the reason at the top of this module.
/// It is its own type because "the keys this client is watching" is a
/// value the dev server, the store and the emitted client all pass around,
/// and a bare `Vec<String>` at three layers invites one of them to sort it,
/// dedupe it, or treat empty as "everything".
///
/// Empty means **nothing**, never everything. A client that asked for no
/// keys wants no traffic; reading empty as a wildcard would turn a
/// program with no durable state into one subscribed to every write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keys(Vec<String>);

impl Keys {
    pub fn new(keys: impl IntoIterator<Item = impl Into<String>>) -> Keys {
        let mut keys: Vec<String> = keys.into_iter().map(Into::into).collect();
        keys.sort();
        keys.dedup();
        Keys(keys)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0
            .binary_search_by(|held| held.as_str().cmp(key))
            .is_ok()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

#[derive(Default)]
struct Inner {
    /// `(keys, sender)`. A dead sender is reaped on the next publish,
    /// which is the same way the dev server reaps closed browser tabs.
    subscribers: Vec<(Keys, Sender<Update>)>,
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
        self.publish_all(vec![update]);
    }

    /// Announce one transaction's writes together.
    ///
    /// The lock is taken once for the whole batch, so a subscriber sees a
    /// transaction's updates contiguously rather than interleaved with
    /// another transaction's. That is not atomicity — a subscriber
    /// rendering each push as it lands still sees the keys arrive one at a
    /// time, and [`crate::DurableStore::apply`] says so — but interleaving
    /// two transactions would mean a client could reconstruct a state that
    /// never existed, and this costs one lock acquisition to prevent.
    pub fn publish_all(&self, updates: Vec<Update>) {
        if updates.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().expect("durable store fanout poisoned");
        for update in updates {
            inner.latest = update.seq;
            if inner.backlog.len() == BACKLOG {
                inner.backlog.pop_front();
            }
            inner.backlog.push_back(update.clone());
            inner.subscribers.retain(|(keys, tx)| {
                !keys.contains(&update.key) || tx.send(update.clone()).is_ok()
            });
        }
    }

    pub fn latest(&self) -> Seq {
        self.inner
            .lock()
            .expect("durable store fanout poisoned")
            .latest
    }

    /// Subscribe to `keys`, resuming after `since`.
    ///
    /// Registration and the backlog snapshot happen under one lock, so a
    /// write landing mid-subscribe is either in the replayed tail or on
    /// the channel — never in neither, which is the only way it could be
    /// lost.
    pub fn subscribe(&self, keys: &Keys, since: Option<Seq>) -> Subscription {
        let (tx, rx) = mpsc::channel();
        let mut inner = self.inner.lock().expect("durable store fanout poisoned");
        inner.subscribers.push((keys.clone(), tx));

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
                    .filter(|update| update.seq > seen && keys.contains(&update.key))
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

    /// Every key these tests write to.
    ///
    /// Spelled out rather than expressed as a wildcard, because there is
    /// no wildcard: a client asks for the durable keys its program
    /// declares, and this is that list for the program these tests stand
    /// in for.
    fn everything() -> Keys {
        Keys::new(["visits", "a", "b", "session/7/cart"])
    }

    #[test]
    fn a_subscriber_hears_a_write_that_lands_after_it_subscribed() {
        let fanout = Fanout::default();
        let mut window = fanout.subscribe(&everything(), None);
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
        let mut a = fanout.subscribe(&everything(), None);
        let mut b = fanout.subscribe(&everything(), None);
        fanout.publish(update(1, "visits", "1"));
        assert!(a.try_next().is_some());
        assert!(b.try_next().is_some());
    }

    #[test]
    fn a_subscriber_hears_nothing_outside_the_keys_it_asked_for() {
        let fanout = Fanout::default();
        let mut window = fanout.subscribe(&Keys::new(["session/7/cart"]), None);
        fanout.publish(update(1, "visits", "1"));
        assert_eq!(window.try_next(), None);
        fanout.publish(update(2, "session/7/cart", "[]"));
        assert!(window.try_next().is_some());
    }

    #[test]
    fn an_empty_key_set_hears_nothing_rather_than_everything() {
        // The failure mode this rules out is a program with no durable
        // state accidentally subscribing to every write in the store.
        let fanout = Fanout::default();
        let mut window = fanout.subscribe(&Keys::default(), None);
        fanout.publish(update(1, "visits", "1"));
        assert_eq!(window.try_next(), None);
    }

    #[test]
    fn a_key_set_is_sorted_and_deduplicated_on_the_way_in() {
        // `contains` binary-searches, so an unsorted set would silently
        // miss keys — and the compiler emits this list from a `BTreeSet`
        // it has already ordered, which is exactly how such a bug hides.
        let keys = Keys::new(["visits", "answers", "visits"]);
        assert_eq!(keys.iter().collect::<Vec<_>>(), vec!["answers", "visits"]);
        assert!(keys.contains("visits"));
        assert!(keys.contains("answers"));
        assert!(!keys.contains("vis"), "a key set is not a prefix match");
    }

    #[test]
    fn a_fresh_subscriber_is_told_nothing_it_already_has() {
        let fanout = Fanout::default();
        fanout.publish(update(1, "visits", "1"));
        let mut window = fanout.subscribe(&everything(), None);
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

        let mut window = fanout.subscribe(&everything(), Some(Seq(1)));
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
        let mut window = fanout.subscribe(&everything(), Some(Seq(1)));
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
        let mut window = fanout.subscribe(&everything(), Some(Seq(1)));
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
        let mut window = fanout.subscribe(&everything(), Some(Seq(4)));
        assert_eq!(window.try_next(), Some(Event::Resync { seq: Seq(9) }));
    }

    #[test]
    fn a_dropped_subscriber_is_reaped_on_the_next_write() {
        let fanout = Fanout::default();
        let window = fanout.subscribe(&everything(), None);
        assert_eq!(fanout.subscribers(), 1);
        drop(window);
        fanout.publish(update(1, "visits", "1"));
        assert_eq!(fanout.subscribers(), 0);
    }
}
