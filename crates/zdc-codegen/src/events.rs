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
        _ => return None,
    };
    let access = accessor(payload, field)?;
    Some(format!("({parameter}) => {setter}({parameter}.{access})"))
}

/// The event the two-way sugar listens for, per §16.3.6.
pub fn two_way_event(attribute: &str) -> Option<&'static str> {
    match attribute {
        "value" => Some("input"),
        "checked" => Some("change"),
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
        for (event, payload) in zdc_types::EVENTS {
            for (field, _) in payload.fields() {
                assert!(
                    accessor(*payload, field).is_some(),
                    "`on {event}` declares `{field}` with nowhere to read it"
                );
            }
        }
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
}
