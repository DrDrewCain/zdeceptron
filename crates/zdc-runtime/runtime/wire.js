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
//
// # The compatibility rule — decided 2026-08-09 (#144), `DECISIONS.md` §6
//
// There is no version on the wire, and there is not going to be one.
// Within a build there are not two ends to disagree: one `zdc build` emits
// the client bundle, the server handlers and the store adapter from one
// run over one program, and all three link this file. So compatibility is
// an obligation on the compiler rather than a field in the payload:
//
//   1. The encoding of a shape that has ever been persisted may not
//      change. Durable values are stored encoded, so a change to how a
//      shape is written silently reinterprets an older build's data. That
//      is not hypothetical — it is #204, above, where a `[1]` was stored
//      as `{"base":[],"item":1,"flat":null}` and nothing raised a word.
//   2. A new shape takes a new `$`-prefixed marker, for the reason `$map`
//      is unambiguous: `$` is in neither `XID_Start` nor `XID_Continue`,
//      so no record field can ever collide with one.
//   3. A disagreement is a *named* failure at the decode site and never a
//      coercion. `decode` throws on every malformed `$map` below, and
//      `rpc.js` turns a decoder rejection into `Failed(Rejected)` — a
//      closed `FailureCode` variant the program can match on, not a
//      console message.
//
// The rejected alternative is a version integer in an envelope around
// every request and every stored value. It would have to be threaded
// through `rpc.js`, `store.js`, every emitted endpoint and every deploy
// adapter; it protects the case that cannot happen (two halves of one
// build disagreeing) rather than the one that can (an older build's stored
// bytes); and "are these the same format" is not the question a stale
// durable value poses. That question is "is this the shape this program's
// type says it is", it is answered by a digest of the declared shape
// stored beside the value, and it belongs in the store rather than here.
// Until that exists, rule 1 is a rule a human keeps and the compiler does
// not check — which `DECISIONS.md` §6 states as the gap it leaves.

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
    for (const [key, item] of value) entries.push([encode(key), encode(item)]);
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

/** A ZD value as the JSON text that crosses the wire. */
export function stringify(value) {
  const text = JSON.stringify(encode(value));
  return text === undefined ? 'null' : text;
}

/** JSON text back into a ZD value. */
export function parse(text) {
  return decode(JSON.parse(text));
}
