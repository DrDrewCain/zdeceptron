// Tests for the wire format. Run: `cargo test -p zdc-runtime`
//
// `wire.js` had no tests at all, which for the file that decides what
// survives the trip to the store is the wrong place to have none. The
// module's own argument for why its marker is unambiguous is an argument
// about ZD *identifiers* — no record field can be called `$map`, because
// `$` is in neither of the lexer's identifier classes. That argument holds
// for values a ZD program authored. It does not hold for values a ZD
// program *received*: `rpc.js` decodes whatever an endpoint answers with
// and `store.js` decodes whatever a live-sync frame carries, and neither is
// under the program's control.
//
// `test` and `assert` come from the harness; `encode`, `decode`,
// `stringify` and `parse` come from wire.js evaluated in the same scope.

const roundTrip = (value) => parse(stringify(value));

test('the four scalar shapes survive the trip', () => {
  assert.equal(roundTrip(42), 42);
  assert.equal(roundTrip(-1.5), -1.5);
  assert.equal(roundTrip('ada'), 'ada');
  assert.equal(roundTrip(true), true);
  assert.equal(roundTrip(false), false);
  assert.equal(roundTrip(null), null);
});

test('undefined becomes null, so absent and empty agree', () => {
  assert.equal(stringify(undefined), 'null');
  assert.equal(encode(undefined), null);
  assert.deepEqual(encode({ a: undefined }), { a: null });
});

test('a Map survives, which JSON.stringify alone does not', () => {
  const original = new Map([
    ['ada', 1],
    ['grace', 2],
  ]);
  assert.equal(JSON.stringify(original), '{}', 'the bug this file exists to fix');
  assert.equal(stringify(original), '{"$map":[["ada",1],["grace",2]]}');

  const back = roundTrip(original);
  assert.ok(back instanceof Map, 'a map must come back a map');
  assert.equal(back.get('ada'), 1);
  assert.equal(back.get('grace'), 2);
  assert.equal(back.size, 2);
});

test('an empty map is distinguishable from an empty record', () => {
  assert.equal(stringify(new Map()), '{"$map":[]}');
  assert.equal(stringify({}), '{}');
  assert.ok(roundTrip(new Map()) instanceof Map);
  assert.ok(!(roundTrip({}) instanceof Map));
});

test('a map whose own key is the marker is not confused for the marker', () => {
  const original = new Map([['$map', 7]]);
  assert.equal(stringify(original), '{"$map":[["$map",7]]}');
  const back = roundTrip(original);
  assert.ok(back instanceof Map);
  assert.equal(back.get('$map'), 7);
  assert.equal(back.size, 1);
});

test('maps nest, as keys and as values', () => {
  const inner = new Map([['k', 1]]);
  const original = new Map([['outer', inner]]);
  const back = roundTrip(original);
  assert.ok(back.get('outer') instanceof Map);
  assert.equal(back.get('outer').get('k'), 1);

  const listOfMaps = roundTrip([new Map([['a', 1]]), new Map([['b', 2]])]);
  assert.equal(listOfMaps.length, 2);
  assert.ok(listOfMaps[0] instanceof Map);
  assert.equal(listOfMaps[1].get('b'), 2);
});

test('a record holding a map keeps both shapes', () => {
  const back = roundTrip({ name: 'ada', scores: new Map([['maths', 9]]) });
  assert.equal(back.name, 'ada');
  assert.ok(back.scores instanceof Map);
  assert.equal(back.scores.get('maths'), 9);
});

test('a choice rides as an ordinary object and recurses', () => {
  const back = roundTrip({ tag: 'Ready', fields: [new Map([['a', 1]])] });
  assert.equal(back.tag, 'Ready');
  assert.ok(back.fields[0] instanceof Map);
});

// --- what a value says about its own JSON form -----------------------------
//
// `encode` walks before `JSON.stringify` is called, so `JSON.stringify`
// never meets the original object and every `toJSON` in the program was
// silently overridden. `append` compiles to a chain of links carrying a
// `toJSON` that flattens it, and a durable `[1]` was stored as
// `{"base":[],"item":1,"flat":null}` (#204). The rule is general rather than
// a third branch beside `$map`, so the next type with a `toJSON` does not
// break the same way.

// The shape `zdc-codegen` emits for `append`, near enough for this file:
// links, a cached flattening, and a `toJSON` that hands back the list.
class Chain {
  constructor(base, item) {
    this.base = base;
    this.item = item;
    this.flat = null;
  }
  toJSON() {
    if (!this.flat) {
      const base = this.base instanceof Chain ? this.base.toJSON() : this.base;
      this.flat = [...base, this.item];
    }
    return this.flat;
  }
}

test('a value with toJSON is encoded as the form it declares', () => {
  const chain = new Chain(new Chain([], 1), 2);
  assert.equal(stringify(chain), '[1,2]');
  assert.deepEqual(encode(chain), [1, 2]);
  assert.deepEqual(roundTrip(chain), [1, 2]);
});

test('a declared form is walked in turn, so a map inside one survives', () => {
  const holder = { toJSON: () => ({ scores: new Map([['ada', 1]]) }) };
  assert.equal(stringify(holder), '{"scores":{"$map":[["ada",1]]}}');
  assert.ok(roundTrip(holder).scores instanceof Map);
});

test('a declared form is honoured at any depth', () => {
  const bag = { tags: new Chain([], 1), size: 1 };
  assert.equal(stringify(bag), '{"tags":[1],"size":1}');
  assert.equal(stringify([new Chain([], 7)]), '[[7]]');
});

test('a toJSON that returns its receiver is walked instead of looping', () => {
  class SelfReferring {
    constructor() {
      this.a = 1;
    }
    toJSON() {
      return this;
    }
  }
  // Structurally, which is what every value got before this rule existed.
  // The alternative is recursing on the same object for ever, and a codec
  // that hangs is worse than one that ignores an empty declaration.
  assert.equal(stringify(new SelfReferring()), '{"a":1}');
  assert.deepEqual(encode(new SelfReferring()), { a: 1 });
});

test('a map is unaffected, having no toJSON to consult', () => {
  assert.equal(typeof new Map().toJSON, 'undefined', 'the reason this file exists');
  assert.equal(stringify(new Map([['ada', 1]])), '{"$map":[["ada",1]]}');
});

// --- what a malformed or hostile payload does ------------------------------
//
// Every one of these was a silent conversion before: a non-array `$map`
// became an empty map, sibling fields vanished, and a malformed pair was
// skipped. Silence is the one failure mode a persistence format must not
// have — it is the exact bug the module was written to fix, arriving from
// the other direction.

test('a $map that is not an array is refused rather than emptied', () => {
  for (const payload of [{ $map: 'junk' }, { $map: 7 }, { $map: null }, { $map: {} }]) {
    let threw = false;
    try {
      decode(payload);
    } catch (e) {
      threw = true;
      assert.ok(String(e.message).includes('array'), 'the message must say what was expected');
    }
    assert.ok(threw, 'refused: ' + JSON.stringify(payload));
  }
});

test('a $map carrying sibling fields is refused rather than losing them', () => {
  let threw = false;
  try {
    decode({ $map: [], name: 'ada' });
  } catch (e) {
    threw = true;
    assert.ok(String(e.message).includes('name'), 'the message must name what would be lost');
  }
  assert.ok(threw, 'a record is not a map with extra fields, and neither is a map');
});

test('a $map entry that is not a pair is refused rather than skipped', () => {
  for (const entries of [[['a']], [['a', 1, 2]], ['a'], [null]]) {
    let threw = false;
    try {
      decode({ $map: entries });
    } catch (e) {
      threw = true;
    }
    assert.ok(threw, 'refused: ' + JSON.stringify(entries));
  }
});

// The one confusion that remains, and it is a property of the format
// rather than a defect in this file: a payload shaped like the marker
// decodes as a map, because that is what the marker means. A ZD program
// cannot author such a record — `$` is in neither identifier class — so
// this can only arrive from outside, and outside is exactly where the three
// checks above now stop the malformed cases.
test('a payload shaped like the marker decodes as the map it claims to be', () => {
  const back = decode({ $map: [['a', 1]] });
  assert.ok(back instanceof Map, 'the marker means what it says');
  assert.equal(back.get('a'), 1);
});

test('the marker is inert everywhere except as a field name', () => {
  assert.equal(roundTrip('$map'), '$map');
  assert.deepEqual(roundTrip(['$map']), ['$map']);
  assert.deepEqual(roundTrip({ notmap: '$map' }), { notmap: '$map' });
});
