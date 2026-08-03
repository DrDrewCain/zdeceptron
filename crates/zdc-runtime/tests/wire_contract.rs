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

#[test]
fn malformed_map_entries_are_ignored_without_inventing_values() {
    let mut sandbox = wire();
    assert_eq!(
        sandbox
            .text(
                "(() => { const m = decode({$map: [['ok', 1], ['short'], 'bad', ['also', 2, 3]]}); return m instanceof Map && m.size === 1 && m.get('ok') === 1; })()",
            )
            .unwrap(),
        "true"
    );
}

#[test]
fn a_non_array_map_payload_decodes_as_an_empty_map() {
    let mut sandbox = wire();
    assert_eq!(
        sandbox
            .text("(() => { const m = decode({$map: 'bad'}); return m instanceof Map && m.size === 0; })()")
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
