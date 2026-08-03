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

/** A ZD value as JSON-representable data. */
export function encode(value) {
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
    for (const field of Object.keys(value)) out[field] = encode(value[field]);
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
    for (const field of Object.keys(value)) out[field] = decode(value[field]);
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
