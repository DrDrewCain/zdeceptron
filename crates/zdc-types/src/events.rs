//! What each event carries, and which events there are.
//!
//! This is the *type* half of the event table; `zdc-codegen` owns the
//! *DOM* half — which property of the browser's event object each field
//! reads. The split is the one §16.3.6 already makes for elements, and it
//! is kept honest the same way: a parity test asserts the two tables name
//! the same fields for the same payload.
//!
//! # Why the set is closed
//!
//! An open event type would let a program write `on gamepadconnected` and
//! reach whatever the browser put on the object, which is the reach a
//! wrapper library has. It also gives up every property this language
//! exists for: an open payload has no field types, so `press.key + 1` is
//! not an error, and — the part that matters — an open payload has no
//! declared provenance, so the integrity lattice (§18.1) would have
//! nothing to attach an untrusted label to.
//!
//! The set is therefore closed at the events the built-in elements can
//! raise, and it grows by adding a row here, in the emitter's table, and
//! in one test. The cost is stated rather than argued away: a program
//! cannot listen for an event this table does not name, and until §14E's
//! FFI can own a DOM node there is no escape hatch either.

use crate::ty::Type;

/// The shape of the value a handler's binder receives.
///
/// Five payloads over eight events, rather than eight payloads: `keydown`
/// and `keyup` differ in *when* they fire and not in what they carry, and
/// a type per event would be five copies of the same field list for a
/// distinction the type system cannot use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPayload {
    /// A press: where it landed, and which modifiers were held.
    Pointer,
    /// A field's contents at the moment the event fired.
    Edit,
    /// A keystroke: which key, and which modifiers were held.
    Key,
    /// Focus arriving at or leaving a field, carrying the field's value —
    /// which is what "commit on blur" needs and nothing else supplies.
    Focus,
    /// A form submission. It carries nothing: a submit event's target is
    /// the form, not a field, so there is no value to read. See the
    /// crate report for what makes this useful and why that is not here.
    Submit,
}

impl EventPayload {
    /// How this payload is named in a diagnostic.
    ///
    /// Not a name a program can write: `Type::from_name` does not produce
    /// one, and there is no `state e is client PointerEvent`. A payload
    /// exists only where a handler bound it.
    pub fn describe(self) -> &'static str {
        match self {
            EventPayload::Pointer => "PointerEvent",
            EventPayload::Edit => "EditEvent",
            EventPayload::Key => "KeyEvent",
            EventPayload::Focus => "FocusEvent",
            EventPayload::Submit => "SubmitEvent",
        }
    }

    /// Every field this payload carries, in the order a diagnostic lists
    /// them.
    pub fn fields(self) -> &'static [(&'static str, Type)] {
        // Every field is a base type, so the tables are constants. The
        // four modifier fields are spelled out in each of the two payloads
        // that carry them rather than shared: `Type` is not `Copy`, and a
        // test below asserts the two lists agree so the repetition cannot
        // drift.
        const POINTER: [(&str, Type); 6] = [
            ("x", Type::Decimal),
            ("y", Type::Decimal),
            ("shift", Type::Truth),
            ("control", Type::Truth),
            ("alt", Type::Truth),
            ("meta", Type::Truth),
        ];
        const EDIT: [(&str, Type); 2] = [("value", Type::Text), ("checked", Type::Truth)];
        const KEY: [(&str, Type); 5] = [
            ("key", Type::Text),
            ("shift", Type::Truth),
            ("control", Type::Truth),
            ("alt", Type::Truth),
            ("meta", Type::Truth),
        ];
        const FOCUS: [(&str, Type); 1] = [("value", Type::Text)];

        match self {
            EventPayload::Pointer => &POINTER,
            EventPayload::Edit => &EDIT,
            EventPayload::Key => &KEY,
            EventPayload::Focus => &FOCUS,
            EventPayload::Submit => &[],
        }
    }

    /// The type of one field, or `None` if this payload has no such field.
    pub fn field(self, name: &str) -> Option<Type> {
        self.fields()
            .iter()
            .find(|(field, _)| *field == name)
            .map(|(_, ty)| ty.clone())
    }
}

/// Every event a built-in element can raise, with the payload it carries.
///
/// The order is the order a diagnostic offers them in.
pub const EVENTS: &[(&str, EventPayload)] = &[
    ("click", EventPayload::Pointer),
    ("input", EventPayload::Edit),
    ("change", EventPayload::Edit),
    ("submit", EventPayload::Submit),
    ("keydown", EventPayload::Key),
    ("keyup", EventPayload::Key),
    ("focus", EventPayload::Focus),
    ("blur", EventPayload::Focus),
];

/// The payload of `event`, or `None` when the language does not know it.
pub fn payload_of(event: &str) -> Option<EventPayload> {
    EVENTS
        .iter()
        .find(|(name, _)| *name == event)
        .map(|(_, payload)| *payload)
}

/// Every event name, for the diagnostic that has to offer the whole set.
pub fn event_names() -> Vec<&'static str> {
    EVENTS.iter().map(|(name, _)| *name).collect()
}

/// The keys `on key "…"` may name that are not a single character.
///
/// Exactly the `KeyboardEvent.key` spellings, because the emitted listener
/// compares against `event.key` and nothing translates. Closed rather than
/// "any string", and the reason is not security: `on key "Esc"` is a
/// listener that never fires, and a browser reports that as silence.
pub const NAMED_KEYS: &[&str] = &[
    "Escape",
    "Enter",
    "Tab",
    "Backspace",
    "Delete",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
];

/// Why a document key handler observes nothing it was not given.
///
/// One wording, held in one place, because a diagnostic, an explanation
/// and a runtime module each implement a part of it, and a second wording
/// is how they come to disagree.
pub const DOCUMENT_KEY_RULE: &str =
    "a document key handler learns only that the key it named itself was pressed while no \
     editable element had focus";

/// Whether `key` is a key `on key "…"` may name.
///
/// Two admissible shapes, and the second carries the weight: **exactly one
/// character**. `"gg"` is not a key, it is two, and a program meaning a
/// chord is asking for something this construct does not have. `" "` is
/// one character and is therefore the space bar, which is what a game
/// means by it.
pub fn is_document_key(key: &str) -> bool {
    NAMED_KEYS.contains(&key) || key.chars().count() == 1
}

/// The named key a misspelling most likely meant, for the diagnostic.
///
/// Prefix matching under a case fold rather than an edit distance: the
/// mistakes that actually happen are `"esc"`, `"Esc"`, `"arrowleft"` and
/// `"Escape "`, every one of which is a prefix of the right answer or has
/// one as a prefix.
pub fn suggest_key(written: &str) -> Option<&'static str> {
    let lowered = written.to_lowercase();
    if lowered.is_empty() {
        return None;
    }
    NAMED_KEYS.iter().copied().find(|named| {
        let folded = named.to_lowercase();
        folded.starts_with(&lowered) || lowered.starts_with(&folded)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_the_language_names_has_a_payload() {
        for (name, _) in EVENTS {
            assert!(payload_of(name).is_some(), "{name} has no payload");
        }
        assert!(payload_of("gamepadconnected").is_none());
    }

    /// The three payloads a browser genuinely distinguishes carry the
    /// fields the gap analysis said were unreachable: a key, a coordinate,
    /// and a field's value.
    #[test]
    fn the_payloads_carry_what_a_handler_could_not_reach_before() {
        assert_eq!(
            payload_of("keydown").expect("keydown").field("key"),
            Some(Type::Text)
        );
        assert_eq!(
            payload_of("click").expect("click").field("x"),
            Some(Type::Decimal)
        );
        assert_eq!(
            payload_of("input").expect("input").field("value"),
            Some(Type::Text)
        );
        assert_eq!(
            payload_of("blur").expect("blur").field("value"),
            Some(Type::Text)
        );
    }

    /// A payload is not a bag: reading a field it does not carry is a
    /// question the table answers `None` to, which is what makes
    /// `press.value` on a click an error rather than `undefined`.
    #[test]
    fn a_payload_carries_only_its_own_fields() {
        let pointer = payload_of("click").expect("click");
        assert_eq!(pointer.field("key"), None);
        assert_eq!(pointer.field("value"), None);
        assert_eq!(payload_of("input").expect("input").field("x"), None);
    }

    /// The modifier fields are written out twice, so assert they agree.
    #[test]
    fn both_modifier_bearing_payloads_carry_the_same_four() {
        for modifier in ["shift", "control", "alt", "meta"] {
            assert_eq!(
                EventPayload::Pointer.field(modifier),
                Some(Type::Truth),
                "a pointer event is missing `{modifier}`"
            );
            assert_eq!(
                EventPayload::Key.field(modifier),
                Some(Type::Truth),
                "a key event is missing `{modifier}`"
            );
        }
    }

    #[test]
    fn a_submit_carries_nothing() {
        assert!(payload_of("submit").expect("submit").fields().is_empty());
    }

    /// Both key events and both focus events share one payload, so a
    /// program written against `keydown` reads the same on `keyup`.
    #[test]
    fn events_that_differ_only_in_timing_share_a_payload() {
        assert_eq!(payload_of("keydown"), payload_of("keyup"));
        assert_eq!(payload_of("focus"), payload_of("blur"));
        assert_eq!(payload_of("input"), payload_of("change"));
    }

    /// `key` is not an event, and this is what makes `on key "…"`
    /// unambiguous against `on <event>` without a lookahead.
    #[test]
    fn key_is_not_an_event_name() {
        assert_eq!(payload_of("key"), None);
    }

    /// The two admissible shapes, and the two that are neither.
    #[test]
    fn a_document_key_is_a_named_key_or_exactly_one_character() {
        assert!(is_document_key("Escape"));
        assert!(is_document_key("ArrowLeft"));
        assert!(is_document_key("r"));
        assert!(is_document_key("~"));
        assert!(is_document_key(" "), "the space bar is one character");

        assert!(!is_document_key("Esc"), "not a `KeyboardEvent.key`");
        assert!(!is_document_key("gg"), "two characters is not a key");
        assert!(!is_document_key(""), "nothing is not a key");
        assert!(
            !is_document_key("escape"),
            "`KeyboardEvent.key` is case-sensitive, so a fold here would \
             accept a listener that never fires"
        );
    }

    /// The whole reason the set is closed: the plausible misspelling is
    /// caught and repaired rather than reported as silence at run time.
    #[test]
    fn a_misspelled_key_is_offered_the_one_it_meant() {
        assert_eq!(suggest_key("Esc"), Some("Escape"));
        assert_eq!(suggest_key("esc"), Some("Escape"));
        assert_eq!(suggest_key("arrowleft"), Some("ArrowLeft"));
        assert_eq!(suggest_key("Escape "), Some("Escape"));
        assert_eq!(suggest_key("zzz"), None);
        assert_eq!(suggest_key(""), None, "everything has an empty prefix");
    }

    /// Every named key produces no character in any editable element, so
    /// the rule this file states holds over the whole table rather than
    /// over the entries somebody happened to think about.
    #[test]
    fn no_named_key_is_a_character() {
        for named in NAMED_KEYS {
            assert!(
                named.chars().count() > 1,
                "`{named}` is a single character, so it is a key a field \
                 receives and the named list is the wrong place for it"
            );
        }
    }
}
