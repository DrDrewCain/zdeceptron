// The wire format: how a ZD value survives the trip to the store and back.
//
// # The bug this file exists to fix
//
// `JSON.stringify` cannot represent a JavaScript `Map`. It does not throw,
// it does not warn, it returns `{}`:
//
//     JSON.stringify(new Map([['ada', 1]]))   // → "{}"
//
// A `Map` is what §5.4's `Map of K to V` compiles to — an object would
// coerce every key to a string, which is exactly why it is a `Map` — so
// every `durable Map` wrote an empty object and read nothing back. It
// failed silently, which for a persistence bug is the worst way to fail.
//
// # Why a marker and not an array
//
// `Map` and `record` cannot share an encoding: a record is a plain object
// and a map has to be distinguishable from one, or `decode` cannot know
// which to rebuild. So a map is tagged:
//
//     new Map([['ada', 1]])   ⇄   { "$map": [["ada", 1]] }
//
// `$` cannot appear in a ZD identifier — the lexer's rule is
// `[\p{XID_Start}_][\p{XID_Continue}]*`, and `$` is in neither class — so
// no record field can ever be named `$map` and the marker is unambiguous
// by construction rather than by convention.
//
// # What the four shapes look like on the wire
//
//     Whole, Decimal   number          Text     string
//     Truth            boolean         empty    []  or  {"$map":[]}
//     List of T        array           record   object, fields by name
//     Map of K to V    {"$map":[[k,v]]}         choice   {tag, fields}
//
// A choice is `{ tag, fields }` (see `variant` in `dom.js`) and a record is
// a plain object, so both ride as ordinary JSON objects and recurse.
//
// # One definition, three users
//
// The browser encodes a request body, the adapter decodes it and encodes
// what it stores, and the live-sync stream carries the encoded form
// straight through. Three places, one file — a second copy of these rules
// anywhere is how the two halves come to disagree about what `{}` means.
//
// # Why `encode` consults `toJSON`, and why it is not a second marker
//
// `encode` runs *before* `JSON.stringify` and hands it a value that has
// already been walked. That is what makes the `$map` marker possible, and
// it also means `JSON.stringify` never sees the original object, so every
// `toJSON` in the program was silently defeated, and any type that grew one
// later would have been defeated the same way.
//
// It cost this once already. `append` compiles to a chain of links rather
// than to an array, because appending has to be O(1) or a builder is
// quadratic, and the class carries a `toJSON` that flattens the chain for
// exactly this trip. `encode` walked past it: a link is not a `Map` and
// `Array.isArray` is false for one, so it fell through to the record branch
// and a durable `[1]` was stored as `{"base":[],"item":1,"flat":null}`
// (#204).
//
// The narrow fix would have been a third branch that recognises the link
// class. It was rejected: the mistake is not that this file does not know
// about `append`, it is that walking structurally overrides what a value
// says about its own JSON form, and a third branch leaves that true for the
// fourth type. So `toJSON` is consulted generally and first, which is the
// rule `JSON.stringify` itself follows, and a type that has an opinion
// about its JSON form now gets it honoured at both layers instead of one.
//
// This does not weaken the `$map` marker's argument. A `Map` has no
// `toJSON`, which is the whole reason this file exists, so nothing about
// how a map rides has changed.

/**
 * Which version of this format the bytes are written in.
 *
 * **No compatibility is promised between versions; a mismatch is refused
 * by name** (#144). The rule and its full argument are in
 * `docs/reference.md` §14; what belongs here is why this file is not the
 * place the version travels.
 *
 * The short form of the argument, because it is about `decode` above: a
 * malformed `$map` throws, since this version knows what one looks like;
 * a marker from a later version cannot throw, since refusing it would
 * need the knowledge the older end is missing. It decodes as a record and
 * reaches the program as `Ready` holding a value nobody wrote —
 * `wire_contract.rs` pins that rather than describing it.
 *
 * This is the *format's* version and not the compiler's. It moves when
 * the bytes move: a new marker, a retired one, a different shape for one
 * of §5.4's four types. Most releases do not touch it, which is what
 * makes refusing affordable rather than a broken redeploy every time.
 *
 * # Why it is not an envelope
 *
 * `{"z":1,"v":…}` is the obvious mechanism and it is wrong here, because
 * `stringify` and `parse` are also the *persistence* format —
 * `zdc-host`'s `$wireStringify` writes durable keys with them. Wrapping
 * the value would version every stored value and rewrite every store on
 * upgrade, which is #37's migration question and not this one.
 *
 * So the number rides beside the bytes: a header on the request and the
 * response, and a query parameter on the live-sync subscription, where
 * `EventSource` cannot set a header. `encode` and `decode` are untouched
 * and a stored value is the same bytes it always was.
 */
export const VERSION = 1;

/** The header both ends name the format in. */
export const VERSION_HEADER = 'zd-wire';

/** The subscription parameter, for the transport that has no headers. */
export const VERSION_PARAM = 'wire';

/** A ZD value as JSON-representable data. */
export function encode(value) {
  if (value !== null && typeof value === 'object' && typeof value.toJSON === 'function') {
    const declared = value.toJSON();
    // A `toJSON` that hands back its own receiver has declared nothing, and
    // recursing on it would not terminate. Walking it structurally is what
    // this function did for every value before, so that is what it falls
    // back to rather than throwing on a value it can still encode.
    if (declared !== value) return encode(declared);
  }
  if (value instanceof Map) {
    const entries = [];
    // `forEach` rather than `for…of` — see the engine note in `signal.js`.
    value.forEach((item, key) => entries.push([encode(key), encode(item)]));
    return { $map: entries };
  }
  if (Array.isArray(value)) {
    return value.map(encode);
  }
  if (value !== null && typeof value === 'object') {
    const out = {};
    for (const field of Object.keys(value)) {
      Object.defineProperty(out, field, {
        value: encode(value[field]),
        enumerable: true,
        configurable: true,
        writable: true,
      });
    }
    return out;
  }
  // `undefined` is not JSON. A durable key that holds nothing reads back as
  // absent, and the endpoint applies the declared `starting` value — so
  // sending `null` here is what makes those two agree.
  return value === undefined ? null : value;
}

/** The inverse. */
export function decode(value) {
  if (Array.isArray(value)) {
    return value.map(decode);
  }
  if (value !== null && typeof value === 'object') {
    if (Object.prototype.hasOwnProperty.call(value, '$map')) {
      // Strict, because `decode` does not only run on data this runtime
      // encoded. `rpc.js` decodes whatever an endpoint answers with and
      // `store.js` decodes whatever a live-sync frame carries, and neither
      // is under the program's control. The marker's unambiguity is an
      // argument about ZD *identifiers*, and it says nothing about a
      // payload that is merely shaped like one.
      //
      // Every one of these used to be a silent conversion: a non-array
      // `$map` became an empty map, sibling fields vanished, and a
      // malformed pair was skipped. Silent is the one thing a persistence
      // format must not be — that is the whole reason this file exists.
      const keys = Object.keys(value);
      if (keys.length !== 1) {
        throw new Error(
          `A map on the wire carries only "$map"; this one also carried ${JSON.stringify(
            keys.filter((key) => key !== '$map')
          )}.`
        );
      }
      const entries = value.$map;
      if (!Array.isArray(entries)) {
        throw new Error('A map on the wire is an array of [key, value] pairs.');
      }
      const rebuilt = new Map();
      for (const entry of entries) {
        if (!Array.isArray(entry) || entry.length !== 2) {
          throw new Error('A map entry on the wire is a [key, value] pair.');
        }
        rebuilt.set(decode(entry[0]), decode(entry[1]));
      }
      return rebuilt;
    }
    const out = {};
    for (const field of Object.keys(value)) {
      // Assignment to `__proto__` invokes Object.prototype's legacy
      // setter instead of creating the record field. ZD identifiers may
      // legally spell `__proto__`, so define every field as an own data
      // property and keep the decoded record's prototype unchanged.
      Object.defineProperty(out, field, {
        value: decode(value[field]),
        enumerable: true,
        configurable: true,
        writable: true,
      });
    }
    return out;
  }
  return value;
}

// $dev
/**
 * Assert `encode` left nothing `JSON.stringify` writes as `{}`.
 *
 * **This checks #204's family rather than its instance.** The bug at the
 * top of this file was a `Map` reaching `JSON.stringify`, which does not
 * throw and does not warn: it returns `{}`, so a `durable Map` wrote an
 * empty object and read nothing back. `encode` fixes that for `Map`, and
 * since the `toJSON` change for a type that declares its own JSON form.
 * What neither fixes is the *next* type with the same property, and no
 * static pass anywhere would see it — the value is a JavaScript object
 * either way.
 *
 * So the invariant is checked instead of the case: after `encode`, every
 * object left is an array or a plain object and every leaf is a JSON
 * scalar. A `Map`, a `Set`, a `Date` or any class instance that does not
 * declare a `toJSON` fails here, naming the path to itself, instead of
 * silently becoming `{}` in somebody's store.
 *
 * Development only. A release build runs `JSON.stringify` against the same
 * encoded value with no check in front of it, which is what it has always
 * done.
 *
 * The walk is a worklist rather than recursion, and that is not a style
 * choice: `encode` is already recursive, and a second recursion over the
 * same value doubles the stack a nested value needs. `wire_fuzz.rs`
 * generates values deep enough that it does not fit, so an assertion
 * written the obvious way would fail on values the format carries.
 */
export function assertEncoded(root, path) {
  const pending = [[root, path === '' ? 'the value' : path]];
  while (pending.length > 0) {
    const [value, at] = pending.pop();
    if (value === null) continue;
    const type = typeof value;
    if (type === 'boolean' || type === 'string' || type === 'number') continue;
    // `NaN` and the infinities are deliberately *not* refused here. They
    // are numbers JSON writes as `null`, so they do not survive the trip —
    // but that is the format's own recorded behaviour (#144), asserted by
    // `wire_fuzz.rs`, and an assertion that refused them would be this file
    // changing the format rather than checking it.
    if (type !== 'object') {
      throw new Error(`${at} is a ${type}, which JSON cannot represent.`);
    }
    if (Array.isArray(value)) {
      for (let i = 0; i < value.length; i += 1) pending.push([value[i], `${at}[${i}]`]);
      continue;
    }
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null) {
      const name = (value.constructor && value.constructor.name) || 'class instance';
      throw new Error(
        `${at} is a ${name} after encoding, and JSON.stringify writes that ` +
          `as {} without saying so. Give it a toJSON, or encode it in encode().`
      );
    }
    for (const field of Object.keys(value)) pending.push([value[field], `${at}.${field}`]);
  }
}
// $end

/** A ZD value as the JSON text that crosses the wire. */
export function stringify(value) {
  const encoded = encode(value);
  // $dev
  assertEncoded(encoded, '');
  // $end
  const text = JSON.stringify(encoded);
  return text === undefined ? 'null' : text;
}

/** JSON text back into a ZD value. */
export function parse(text) {
  return decode(JSON.parse(text));
}
