use std::time::Duration;

use zdc_store::{Event, Fanout, Json, Keys, Seq, Update};

fn update(seq: u64, key: &str, value: Option<&str>) -> Update {
    Update {
        seq: Seq(seq),
        key: key.into(),
        value: value.map(Json::from_text),
    }
}

#[test]
fn sequence_numbers_default_increment_order_and_display() {
    assert_eq!(Seq::default(), Seq(0));
    assert_eq!(Seq(0).next(), Seq(1));
    assert!(Seq(9) < Seq(10));
    assert_eq!(Seq(42).to_string(), "42");
}

#[test]
fn key_sets_sort_deduplicate_and_use_exact_membership() {
    let keys = Keys::new(["zeta", "alpha", "zeta", "middle"]);

    assert_eq!(keys.iter().collect::<Vec<_>>(), ["alpha", "middle", "zeta"]);
    assert!(keys.contains("middle"));
    assert!(!keys.contains("mid"));
    assert!(!keys.is_empty());
    assert!(Keys::default().is_empty());
}

#[test]
fn a_fresh_subscription_starts_at_latest_without_replaying_history() {
    let fanout = Fanout::new(Seq(7));
    fanout.publish(update(8, "count", Some("8")));
    let mut subscription = fanout.subscribe(&Keys::new(["count"]), None);

    assert_eq!(fanout.latest(), Seq(8));
    assert_eq!(subscription.seq(), Seq(8));
    assert_eq!(subscription.try_next(), None);
}

#[test]
fn live_updates_are_filtered_by_exact_key_and_include_deletions() {
    let fanout = Fanout::default();
    let mut subscription = fanout.subscribe(&Keys::new(["wanted"]), None);

    fanout.publish(update(1, "other", Some("1")));
    fanout.publish(update(2, "wanted", None));

    assert_eq!(
        subscription.try_next(),
        Some(Event::Update(update(2, "wanted", None)))
    );
    assert_eq!(subscription.try_next(), None);
}

#[test]
fn reconnecting_replays_only_unseen_matching_updates_in_order() {
    let fanout = Fanout::default();
    fanout.publish_all(vec![
        update(1, "a", Some("1")),
        update(2, "b", Some("2")),
        update(3, "a", Some("3")),
    ]);
    let mut subscription = fanout.subscribe(&Keys::new(["a"]), Some(Seq(1)));

    assert_eq!(subscription.seq(), Seq(3));
    assert_eq!(
        subscription.try_next(),
        Some(Event::Update(update(3, "a", Some("3"))))
    );
    assert_eq!(subscription.try_next(), None);
}

#[test]
fn a_cursor_ahead_of_latest_receives_no_replay() {
    let fanout = Fanout::default();
    fanout.publish(update(1, "a", Some("1")));
    let mut subscription = fanout.subscribe(&Keys::new(["a"]), Some(Seq(99)));

    assert_eq!(subscription.seq(), Seq(1));
    assert_eq!(subscription.try_next(), None);
}

#[test]
fn a_cursor_older_than_the_bounded_backlog_is_told_to_resync() {
    let fanout = Fanout::default();
    for seq in 1..=257 {
        fanout.publish(update(seq, "a", Some("1")));
    }
    let mut subscription = fanout.subscribe(&Keys::new(["a"]), Some(Seq(0)));

    assert_eq!(
        subscription.try_next(),
        Some(Event::Resync { seq: Seq(257) })
    );
    assert_eq!(subscription.try_next(), None);
}

#[test]
fn waiting_for_an_idle_subscription_times_out_without_an_error() {
    let fanout = Fanout::default();
    let mut subscription = fanout.subscribe(&Keys::new(["a"]), None);

    assert_eq!(subscription.next_timeout(Duration::ZERO), None);
}

#[test]
fn dead_subscribers_are_reaped_on_the_next_matching_publish() {
    let fanout = Fanout::default();
    let subscription = fanout.subscribe(&Keys::new(["a"]), None);
    assert_eq!(fanout.subscribers(), 1);
    drop(subscription);

    fanout.publish(update(1, "other", Some("1")));
    assert_eq!(
        fanout.subscribers(),
        1,
        "an unrelated write does not contact this subscriber"
    );
    fanout.publish(update(2, "a", Some("2")));
    assert_eq!(fanout.subscribers(), 0);
}

#[test]
fn publishing_an_empty_batch_changes_nothing() {
    let fanout = Fanout::new(Seq(12));
    let mut subscription = fanout.subscribe(&Keys::new(["a"]), None);

    fanout.publish_all(Vec::new());

    assert_eq!(fanout.latest(), Seq(12));
    assert_eq!(subscription.try_next(), None);
}
