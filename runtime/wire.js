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
      const entries = value.$map;
      const rebuilt = new Map();
      if (Array.isArray(entries)) {
        for (const entry of entries) {
          if (Array.isArray(entry) && entry.length === 2) {
            rebuilt.set(decode(entry[0]), decode(entry[1]));
          }
        }
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
