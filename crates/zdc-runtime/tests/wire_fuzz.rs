//! A generative harness for the wire format, at values nobody would write.
//!
//! `tests/wire.rs` runs a suite of cases somebody thought of. This file runs
//! the cases nobody thought of, which is the half that found the bug the
//! format already had: a `$map` payload that was merely *shaped* like a map
//! became an empty map, sibling fields vanished, and a malformed pair was
//! skipped — silently, in a persistence format, which is the one place
//! silence is not survivable.
//!
//! **The property is stability of the encoded form, not equality of the
//! value.** `decode` rebuilds records and map keys as fresh objects, so a
//! decoded value is never `===` its original and a `Map` keyed by an object
//! cannot be looked up with the key that built it. Comparing the *encoded*
//! forms across the trip asks the question that matters — did anything
//! change on the way — without asserting an identity the format never
//! promised.
//!
//! Three properties, on generated input:
//!
//! * **Round trip.** `stringify(parse(stringify(v)))` is `stringify(v)`.
//!   A dropped field, a map flattened to `{}`, a number reshaped by the
//!   JSON layer: each of those breaks this and nothing else catches them.
//! * **Loud refusal.** Every malformed `$map` throws, wherever it is
//!   buried. The alternative is the silent conversion described above.
//! * **Idempotence on foreign text.** `decode` runs on payloads this
//!   runtime did not encode — `rpc.js` decodes whatever an endpoint
//!   answers, `store.js` whatever a live-sync frame carries. Whatever the
//!   first parse settles on, a second trip must not move.
//!
//! Depth and breadth are bounded on purpose. A stack overflow inside the
//! engine raises `SIGABRT`, which no `catch_unwind` contains and which
//! would take this binary down rather than fail a case — the recursion
//! bound is the parser's problem (#161's other half) and not something this
//! file can assert without becoming the crash it is testing for.
//!
//! Running longer: `ZDC_FUZZ_CASES=200000 cargo test -p zdc-runtime --test
//! wire_fuzz`. The default is sized to finish in about a second.

use boa_engine::{Context, Source};

/// Remove ES module syntax so the module evaluates as one script.
///
/// Same two lines as `wire.rs`. Written out rather than shared: a test
/// binary is its own crate, and this is the whole of what it takes.
fn flatten(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The generator and the three properties.
///
/// Seeded and deterministic: a failure reports the seed that produced it,
/// and re-running that seed reproduces the value exactly. A corpus that
/// cannot be reproduced from its seed is an anecdote.
const FUZZ: &str = r#"
function Rng(seed) { this.s = (seed >>> 0) || 1; }
Rng.prototype.next = function () {
  let x = this.s;
  x ^= x << 13; x >>>= 0;
  x ^= x >>> 17;
  x ^= x << 5;  x >>>= 0;
  this.s = x;
  return x;
};
Rng.prototype.below = function (n) { return n === 0 ? 0 : this.next() % n; };
Rng.prototype.pick = function (a) { return a[this.below(a.length)]; };

// Values chosen for what they do to a JSON layer rather than for variety:
// the marker key itself, the prototype key the format defines a rule for,
// `toJSON` because `encode` consults it first, the numbers JSON cannot
// represent, and the strings that need escaping.
const ATOMS = [
  0, 1, -1, 3.5, -0, 1e308, -1e308, 5e-324, NaN, Infinity, -Infinity,
  '', 'a', '$map', '__proto__', 'toJSON', 'constructor', '"', '\\', '\u0000',
  '\u202e', '😀', true, false, null, undefined,
];
// Field names are drawn from what §4.6 actually admits —
// `[\p{XID_Start}_][\p{XID_Continue}]*` — because a generator that invents
// field names the language cannot spell tests a program nobody can write.
// `$map` is the case that matters: `$` is in neither XID class, so a ZD
// record can never carry the marker as a field, which is the whole
// argument the marker rests on. It stays in `ATOMS` as a string *value*,
// where it is legal and must survive, and the malformed-payload battery
// below covers `$map` arriving from something that is not this runtime.
// `__proto__`, `toJSON` and `constructor` are all legal identifiers here
// and are the three that collide with JavaScript rather than with ZD.
const FIELDS = ['a', 'b', '__proto__', 'toJSON', 'constructor', 'Ключ', '_0'];

// Map keys are scalars the encoded form can still tell apart, which is
// narrower than the values a map can hold — deliberately, and the gap is
// worth stating because the fuzz found it.
//
// A `Map` distinguishes its keys by SameValueZero; the wire distinguishes
// them by their JSON. Those disagree for every value JSON cannot spell:
// `undefined`, `NaN`, `Infinity` and `-Infinity` all encode to `null`, so
// a map holding any two of them as keys arrives with one entry where it
// left with two — silently, which is the one thing this format must not
// do. Objects and arrays disagree the same way: two structurally equal
// records are two keys going in and one coming back.
//
// Whether a ZD program can *build* such a map is a language question and
// not this file's to answer, so the generator stays inside the keys the
// format is known to round-trip and the collapse is recorded against the
// wire format's compatibility rule (#144) rather than asserted here.
// Asserting it either way would be this test deciding the language.
const KEYS = [0, 1, -1, 3.5, 1e308, '', 'a', '$map', '__proto__', 'toJSON', '"', true, false, null];

function value(rng, depth) {
  if (depth <= 0 || rng.below(3) === 0) return rng.pick(ATOMS);
  switch (rng.below(3)) {
    case 0: {
      const out = [];
      for (let i = rng.below(4); i > 0; i--) out.push(value(rng, depth - 1));
      return out;
    }
    case 1: {
      const out = {};
      for (let i = rng.below(4); i > 0; i--) {
        Object.defineProperty(out, rng.pick(FIELDS), {
          value: value(rng, depth - 1),
          enumerable: true, configurable: true, writable: true,
        });
      }
      return out;
    }
    default: {
      const out = new Map();
      for (let i = rng.below(4); i > 0; i--) {
        out.set(rng.pick(KEYS), value(rng, depth - 1));
      }
      return out;
    }
  }
}

// Every shape the marker can be malformed into. Each one used to convert
// silently; each one must now throw, and must still throw when it is buried
// rather than at the root.
const MALFORMED = [
  '{"$map":{}}',
  '{"$map":null}',
  '{"$map":"pairs"}',
  '{"$map":7}',
  '{"$map":[],"x":1}',
  '{"x":1,"$map":[]}',
  '{"$map":[1]}',
  '{"$map":[[1]]}',
  '{"$map":[[1,2,3]]}',
  '{"$map":[[]]}',
  '{"$map":[null]}',
  '{"$map":[[1,2],[3]]}',
];

function fuzz(cases) {
  const failures = [];
  const rng = new Rng(0x5eed);

  for (let i = 0; i < cases; i++) {
    const seed = rng.next();
    const v = value(new Rng(seed), 5);

    // Round trip. The encoded form is what crosses the wire, so the
    // encoded form is what has to survive it.
    let before, after;
    try {
      before = stringify(v);
      after = stringify(parse(before));
    } catch (e) {
      failures.push('seed ' + seed + ' threw on a value it produced: ' + e);
      continue;
    }
    if (before !== after) {
      failures.push('seed ' + seed + ' did not survive: ' + before + ' -> ' + after);
      continue;
    }

    // Idempotence on text this runtime did not write. A second trip must
    // not move what the first one settled on.
    let third;
    try {
      third = stringify(parse(after));
    } catch (e) {
      failures.push('seed ' + seed + ' threw on its own output: ' + e);
      continue;
    }
    if (third !== after) {
      failures.push('seed ' + seed + ' is not idempotent: ' + after + ' -> ' + third);
    }
  }

  // Loud refusal, at the root and buried, in an array and in a record.
  for (const bad of MALFORMED) {
    for (const [where, text] of [
      ['root', bad],
      ['in an array', '[' + bad + ']'],
      ['in a record', '{"f":' + bad + '}'],
      ['under a map', '{"$map":[[1,' + bad + ']]}'],
    ]) {
      let threw = false;
      try { parse(text); } catch (e) { threw = e instanceof Error; }
      if (!threw) {
        failures.push('a malformed map was accepted ' + where + ': ' + text);
      }
    }
  }

  // A well-formed map still decodes, or the check above would pass by
  // refusing everything.
  const ok = parse('{"$map":[[1,2],[3,4]]}');
  if (!(ok instanceof Map) || ok.size !== 2 || ok.get(1) !== 2) {
    failures.push('a well-formed map did not decode');
  }

  // `__proto__` is a legal ZD identifier, so it must land as an own field
  // and leave the prototype alone.
  const poisoned = parse('{"__proto__":{"polluted":1}}');
  if (Object.getPrototypeOf(poisoned) !== Object.prototype) {
    failures.push('decode moved a record\'s prototype');
  }
  if (({}).polluted !== undefined) {
    failures.push('decode polluted Object.prototype');
  }
  if (!Object.prototype.hasOwnProperty.call(poisoned, '__proto__')) {
    failures.push('decode dropped a __proto__ field instead of keeping it');
  }

  return failures.join('\n');
}
"#;

#[test]
fn generated_values_survive_the_wire_and_malformed_ones_are_refused() {
    let cases: usize = std::env::var("ZDC_FUZZ_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_000);

    let mut context = Context::default();
    for (what, source) in [
        ("wire.js", flatten(zdc_runtime::WIRE_JS)),
        ("the fuzz harness", FUZZ.to_string()),
    ] {
        context
            .eval(Source::from_bytes(source.as_bytes()))
            .unwrap_or_else(|e| panic!("{what} failed to evaluate: {e}"));
    }

    let report = context
        .eval(Source::from_bytes(format!("fuzz({cases})").as_bytes()))
        .expect("running the fuzz")
        .to_string(&mut context)
        .expect("the report is a string")
        .to_std_string_escaped();

    assert!(
        report.is_empty(),
        "the wire format did not hold on {cases} generated values:\n{report}"
    );
}
