//! Where each event payload field lives on the browser's event object.
//!
//! `zdc-types` owns the same table's *type* half — which fields a payload
//! has, and of what type. This owns its *DOM* half, and the two are kept
//! apart for the reason §16.3.6 keeps the element tables apart: one says
//! what a program may write, the other says what that becomes. The parity
//! test in `tests/event_parity.rs` is what stops them drifting.
//!
//! Every accessor here is a plain property read off the event, so a
//! payload costs nothing at runtime: there is no wrapper object, no
//! allocation, and no copy. `press.key` *is* `press.key`.

use zdc_types::EventPayload;

/// The property path `field` reads on a browser event of this payload.
///
/// `None` means the type half knows a field this one does not, which is
/// the drift the parity test exists to catch.
pub fn accessor(payload: EventPayload, field: &str) -> Option<&'static str> {
    let modifier = match field {
        "shift" => Some("shiftKey"),
        "control" => Some("ctrlKey"),
        "alt" => Some("altKey"),
        "meta" => Some("metaKey"),
        _ => None,
    };
    match payload {
        EventPayload::Pointer => match field {
            // Viewport coordinates rather than page ones: a handler that
            // positions something reads the same frame the element is laid
            // out in, and `pageX` differs from it by the scroll offset.
            "x" => Some("clientX"),
            "y" => Some("clientY"),
            _ => modifier,
        },
        EventPayload::Key => match field {
            "key" => Some("key"),
            _ => modifier,
        },
        // The value is read off the target rather than off the event: the
        // DOM puts it there, and this is the same access §16.3.6's own
        // two-way row already writes.
        EventPayload::Edit => match field {
            "value" => Some("target.value"),
            "checked" => Some("target.checked"),
            // Reachable through `FileInput`'s binding and through no
            // program, for `number`'s reason one line down:
            // `EventPayload::Edit::fields` does not declare it, so
            // `e.files` is not a field a handler can read. A `FileList`
            // is not a value this language has — it is not a `List`,
            // nothing can be shown, and indexing one would hand a program
            // a `File` object with no type — so the only thing that may
            // touch it is the compiler's own binding, which takes one
            // name out of it and drops the rest.
            "files" => Some("target.files"),
            // Reachable through the numeric two-way sugar and through no
            // program: `EventPayload::Edit::fields` does not declare it,
            // so `e.number` is not a field a handler can read. That is
            // deliberate. `valueAsNumber` is `NaN` on every input whose
            // type is not numeric, and a payload field that is a number on
            // some elements and `NaN` on others is a field nobody can
            // reason about. `Slider` is the one element that binds it, and
            // its type makes the value a number by construction.
            "number" => Some("target.valueAsNumber"),
            _ => None,
        },
        EventPayload::Focus => match field {
            "value" => Some("target.value"),
            _ => None,
        },
        EventPayload::Submit => None,
    }
}

/// The listener body the two-way sugar wires for a bound attribute.
///
/// `Input name` is `on input with e / set name to e.value` written by the
/// compiler, and this is the one place that knows so. Sharing it with the
/// general path is what keeps §14B.5's binding from being a second
/// implementation of an event payload — the accessor comes from the table
/// above either way, so the two cannot disagree about what `value` means.
pub fn two_way_listener(attribute: &str, parameter: &str, setter: &str) -> Option<String> {
    let (payload, field) = match attribute {
        "value" => (EventPayload::Edit, "value"),
        "checked" => (EventPayload::Edit, "checked"),
        // The DOM attribute is still `value`; what differs is which
        // property the listener reads back out of the event. `Slider`
        // binds a number, and `target.value` is the text of one.
        "valueAsNumber" => (EventPayload::Edit, "number"),
        // `NumberInput` and `DateInput`: the same property read, wrapped.
        // A field with nothing usable in it reports `NaN`, which is not a
        // value this language has, and `$optionalNumber` — a preamble
        // helper rather than a runtime export, for the reason
        // `intrinsics.rs` gives — is where that becomes `None`.
        "valueAsOptionalNumber" => (EventPayload::Edit, "number"),
        // `FileInput`. The read is off the *target* rather than off the
        // event, exactly as `value` and `checked` are, but the property
        // is `files` — a `FileList`, which is not a value this language
        // has — so what the listener writes is the one field of the one
        // file that already is one. `$chosenName` is where that happens,
        // and `intrinsics.rs` says what it leaves behind.
        "files" => (EventPayload::Edit, "files"),
        _ => return None,
    };
    let access = accessor(payload, field)?;
    let read = match attribute {
        "valueAsOptionalNumber" => format!("$optionalNumber({parameter}.{access})"),
        "files" => format!("$chosenName({parameter}.{access})"),
        _ => format!("{parameter}.{access}"),
    };
    Some(format!("({parameter}) => {setter}({read})"))
}

/// The event the two-way sugar listens for, per §16.3.6.
pub fn two_way_event(attribute: &str) -> Option<&'static str> {
    match attribute {
        "value" | "valueAsNumber" | "valueAsOptionalNumber" => Some("input"),
        // A file picker fires `input` too in every current browser, but
        // `change` is the one HTML has always specified for it and the
        // one every browser has always fired.
        "checked" | "files" => Some("change"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field the type half declares has somewhere to read it from.
    /// This is the anti-drift assertion; the integration parity test says
    /// the same thing from the other side.
    #[test]
    fn every_typed_field_has_an_accessor() {
        let mut scanned = 0;
        for (event, payload) in zdc_types::EVENTS {
            for (field, _) in payload.fields() {
                scanned += 1;
                assert!(
                    accessor(*payload, field).is_some(),
                    "`on {event}` declares `{field}` with nowhere to read it"
                );
            }
        }
        // An emptied `EVENTS` would pass the loop above having read
        // nothing, so what was read is pinned before it is trusted.
        assert!(
            scanned >= zdc_types::EVENTS.len(),
            "only {scanned} payload fields were checked across {} events",
            zdc_types::EVENTS.len()
        );
    }

    #[test]
    fn a_field_no_payload_declares_has_no_accessor() {
        assert_eq!(accessor(EventPayload::Edit, "key"), None);
        assert_eq!(accessor(EventPayload::Pointer, "value"), None);
        assert_eq!(accessor(EventPayload::Submit, "value"), None);
    }

    /// §16.3.6's two-way row, verbatim. The sugar and a hand-written
    /// `on input with e / set name to e.value` produce the same bytes,
    /// which is the sense in which the binding stopped being special.
    #[test]
    fn the_two_way_sugar_is_the_payload_path() {
        assert_eq!(
            two_way_listener("value", "e", "setName").as_deref(),
            Some("(e) => setName(e.target.value)")
        );
        assert_eq!(
            two_way_listener("checked", "e", "setDone").as_deref(),
            Some("(e) => setDone(e.target.checked)")
        );
        assert_eq!(two_way_event("value"), Some("input"));
        assert_eq!(two_way_event("checked"), Some("change"));
    }

    /// `Slider` binds a number, and the difference is one property read.
    /// `target.value` is text, so a `Whole` signal given `"55"` renders
    /// `551` the moment anything adds one to it.
    #[test]
    fn the_numeric_sugar_reads_the_value_as_a_number() {
        assert_eq!(
            two_way_listener("valueAsNumber", "e", "setLevel").as_deref(),
            Some("(e) => setLevel(e.target.valueAsNumber)")
        );
        assert_eq!(two_way_event("valueAsNumber"), Some("input"));
        // And it is not a field a handler can read, because
        // `valueAsNumber` is `NaN` on every non-numeric input.
        assert!(!EventPayload::Edit
            .fields()
            .iter()
            .any(|(field, _)| *field == "number"));
    }

    /// `NumberInput` and `DateInput` read the same property and wrap it,
    /// because a field with nothing usable in it reports `NaN` and `NaN`
    /// is not a value this language has. The wrapping is one function in
    /// `dom.js`, so it is the same rule `elements.js` applies.
    #[test]
    fn the_optional_numeric_sugar_turns_an_empty_field_into_none() {
        assert_eq!(
            two_way_listener("valueAsOptionalNumber", "e", "setCount").as_deref(),
            Some("(e) => setCount($optionalNumber(e.target.valueAsNumber))")
        );
        assert_eq!(two_way_event("valueAsOptionalNumber"), Some("input"));
        // The unwrapped read is still its own key, so `Slider` did not
        // quietly acquire an `Option`.
        assert_eq!(
            two_way_listener("valueAsNumber", "e", "setLevel").as_deref(),
            Some("(e) => setLevel(e.target.valueAsNumber)")
        );
    }

    /// `FileInput` reads the control's `files` and keeps one name out of
    /// it, on `change` (#47).
    ///
    /// The wrapping is the element's whole type decision made concrete: a
    /// `FileList` is not a value this language has, so what crosses into
    /// the program is `Option of Text` and the `File` objects stay in the
    /// browser. `$chosenName` is a preamble helper for `$optionalNumber`'s
    /// reason.
    #[test]
    fn the_file_sugar_keeps_the_name_and_drops_the_file() {
        assert_eq!(
            two_way_listener("files", "e", "setChosen").as_deref(),
            Some("(e) => setChosen($chosenName(e.target.files))")
        );
        assert_eq!(two_way_event("files"), Some("change"));
        // And it is not a field a handler can read: `e.files` would hand
        // a program a `File` with no type to give it.
        assert!(!EventPayload::Edit
            .fields()
            .iter()
            .any(|(field, _)| *field == "files"));
    }
}
