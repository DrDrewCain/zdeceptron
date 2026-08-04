use zdc_runtime::{Sandbox, WIRE_JS};

fn wire() -> Sandbox {
    let mut sandbox = Sandbox::new();
    sandbox.load(WIRE_JS).expect("wire module loads");
    sandbox
}

#[test]
fn primitive_values_keep_their_json_representations() {
    let mut sandbox = wire();
    assert_eq!(
        sandbox
            .text("[stringify(12), stringify(1.5), stringify(true), stringify('hi'), stringify(null)].join('|')")
            .unwrap(),
        "12|1.5|true|\"hi\"|null"
    );
}

#[test]
fn undefined_has_the_explicit_absent_wire_representation() {
    let mut sandbox = wire();
    assert_eq!(sandbox.text("stringify(undefined)").unwrap(), "null");
    assert_eq!(
        sandbox
            .text("parse(stringify(undefined)) === null")
            .unwrap(),
        "true"
    );
}

#[test]
fn lists_and_records_round_trip_recursively() {
    let mut sandbox = wire();
    assert_eq!(
        sandbox
            .text(
                "stringify(parse(stringify({name: 'Ada', flags: [true, false], nested: {n: 3}})))",
            )
            .unwrap(),
        r#"{"name":"Ada","flags":[true,false],"nested":{"n":3}}"#
    );
}

#[test]
fn maps_use_the_reserved_marker_instead_of_silently_becoming_objects() {
    let mut sandbox = wire();
    assert_eq!(
        sandbox
            .text("stringify(new Map([['ada', 1], ['grace', 2]]))")
            .unwrap(),
        r#"{"$map":[["ada",1],["grace",2]]}"#
    );
}

#[test]
fn map_keys_and_values_round_trip_recursively() {
    let mut sandbox = wire();
    let expression = r#"
        (() => {
          const original = new Map([[new Map([['inner', 1]]), {items: [new Map([[2, 'two']])]}]]);
          const rebuilt = parse(stringify(original));
          const [key, value] = [...rebuilt.entries()][0];
          return rebuilt instanceof Map && key instanceof Map && key.get('inner') === 1 &&
            value.items[0] instanceof Map && value.items[0].get(2) === 'two';
        })()
    "#;
    assert_eq!(sandbox.text(expression).unwrap(), "true");
}

/// A malformed entry is **refused**, not skipped.
///
/// This pinned the opposite until the wire format became a persistence
/// format: a pair of the wrong length was dropped and the map came back
/// shorter than it went in. Skipping is a silent conversion, and a store
/// that silently returns fewer entries than it holds is worse than one
/// that says it cannot read them — the caller has no way to tell a map
/// that was empty from a map that failed to decode. Asserted on the
/// message so that a future `catch` cannot satisfy this by throwing
/// something else.
#[test]
fn a_malformed_map_entry_is_refused_rather_than_skipped() {
    let mut sandbox = wire();
    let error = sandbox
        .text("decode({$map: [['ok', 1], ['short'], 'bad', ['also', 2, 3]]})")
        .expect_err("a malformed pair is refused");
    assert!(
        error
            .message
            .contains("A map entry on the wire is a [key, value] pair."),
        "expected the entry-shape refusal, got: {}",
        error.message
    );
}

/// A `$map` whose payload is not an array is refused for the same reason.
///
/// It decoded to an empty map before, which is the same silent conversion
/// one level up: `{$map: 'bad'}` is not an empty map, it is not a map at
/// all, and answering with one invents a value nobody stored.
#[test]
fn a_non_array_map_payload_is_refused_rather_than_read_as_empty() {
    let mut sandbox = wire();
    let error = sandbox
        .text("decode({$map: 'bad'})")
        .expect_err("a non-array payload is refused");
    assert!(
        error
            .message
            .contains("A map on the wire is an array of [key, value] pairs."),
        "expected the payload-shape refusal, got: {}",
        error.message
    );
}

/// A well-formed map still decodes, so the two refusals above cannot be
/// satisfied by a decoder that refuses everything.
#[test]
fn a_well_formed_map_still_decodes() {
    let mut sandbox = wire();
    assert_eq!(
        sandbox
            .text(
                "(() => { const m = decode({$map: [['ok', 1], ['two', 2]]}); return m instanceof Map && m.size === 2 && m.get('ok') === 1; })()",
            )
            .unwrap(),
        "true"
    );
}

#[test]
fn record_fields_named_like_object_internals_survive_decode() {
    let mut sandbox = wire();
    let expression = r#"
        (() => {
          const value = parse('{"__proto__":{"safe":true},"constructor":"kept","toString":"also kept"}');
          const encoded = JSON.parse(stringify(value));
          return Object.prototype.hasOwnProperty.call(value, '__proto__') &&
            value.__proto__.safe === true && value.constructor === 'kept' &&
            value.toString === 'also kept' && Object.getPrototypeOf(value) === Object.prototype &&
            Object.prototype.hasOwnProperty.call(encoded, '__proto__') && encoded.__proto__.safe === true;
        })()
    "#;
    assert_eq!(sandbox.text(expression).unwrap(), "true");
}

#[test]
fn repeated_round_trips_preserve_the_same_wire_text() {
    let mut sandbox = wire();
    let expression = r#"
        (() => {
          const once = stringify({rows: [new Map([['x', {ok: true}]])], empty: []});
          const twice = stringify(parse(once));
          const three = stringify(parse(twice));
          return once === twice && twice === three;
        })()
    "#;
    assert_eq!(sandbox.text(expression).unwrap(), "true");
}
