// The `remembered` placement: a signal whose store is the browser's own.
//
// `remembered` is to `client` what `durable` is to `server`. A `client`
// cell is one tab's memory and dies with the tab; this one lives in
// `localStorage`, so it survives a reload, it is shared by every tab of
// the same browser on the same origin, and it is shared with nobody else.
// There is no server in this file and no request: the value never leaves
// the browser it was written in.
//
// # Its own module, and why
//
// §16.3.1: a bundle ships nothing it does not use. A program that declares
// no `remembered` state must not download a `localStorage` wrapper, and
// `list.js`, `foreign.js` and `markup.js` are each here for the same
// reason. `linked_runtime` adds this file exactly when the emitter reached
// it, and `wire.js` with it — see below.
//
// # Why `wire.js` and not `JSON.stringify`
//
// `localStorage` holds strings, and the values that go in are ZD values:
// a `Map of Text to Whole`, a `List of Book`, a record. `JSON.stringify`
// silently turns a `Map` into `{}` — the bug `wire.js` was written to fix,
// and its file comment tells the whole story. This is the same trip a
// `durable` value makes to the store and back, so it is the same encoding,
// from the same file. A second set of rules here is how the two halves of
// a program come to disagree about what `{}` means.
//
// # The key, and why it is prefixed
//
// `zd:` plus the signal's source name. The survey that motivated this
// placement found fifteen keys on one origin, all flat and unprefixed
// (`music-open`, `snake-high-score`, `critterdex`), which is fine until
// two things share an origin. The prefix is not a namespace the program
// can choose, because a key it could choose is a key it could compute, and
// a computed key is a way to read a cell the program did not declare.
//
// # What is deliberately not here
//
// **No `try`/`catch` around the read that swallows a parse failure into
// the initial value.** A stored value that will not decode is a real
// disagreement between what is on disk and what the program now expects,
// and the honest ways to handle it are a migration or a versioned key —
// neither of which the language has yet. What this does instead is
// narrow: it falls back only when the entry is *absent*, which is the one
// case that is not a disagreement at all.

import { signal } from './signal.js';
import { parse, stringify } from './wire.js';

/** Where a `remembered` signal's entry lives, given its source name. */
export function rememberedKey(name) {
  return 'zd:' + name;
}

/**
 * A signal backed by `localStorage`.
 *
 * Returns the same `[read, write]` pair `signal` does, so everything
 * downstream — `derived`, `effect`, every binding in `dom.js` — treats one
 * of these exactly as it treats a `client` cell. That is the point: the
 * placement changes where the value is kept and who else can write it, not
 * what a reader does with it.
 *
 * `initial` is the value on a browser that has never run this program.
 * It is not a default the read falls back to on every load: once an entry
 * exists, the entry is the value, which is what "survives the reload"
 * means.
 */
export function remembered(name, initial) {
  const key = rememberedKey(name);
  const store = storage();
  const [read, write] = signal(load(store, key, initial));

  // Another tab of the same browser wrote the same key. `storage` fires in
  // every *other* document on the origin and never in the one that wrote,
  // which is exactly the edge needed and no echo to suppress. Without it
  // "one value per browser" would be false the moment a second tab is
  // open, and the placement's whole claim with it.
  if (store && typeof addEventListener === 'function') {
    addEventListener('storage', (event) => {
      if (!event || event.key !== key) return;
      // A `null` newValue is the entry being removed — by the visitor
      // clearing site data, or by another script. The program's own
      // starting value is what it had before any of this happened, so
      // that is what it goes back to.
      write(event.newValue === null ? initial : parse(event.newValue));
    });
  }

  return [
    read,
    (next) => {
      const value = typeof next === 'function' ? next(read()) : next;
      save(store, key, value);
      return write(value);
    },
  ];
}

/**
 * The browser's `localStorage`, or `null` where there is none.
 *
 * Two hosts have none: the DOM shim the compiler's own tests render
 * against, and a browser in a mode where reading the property throws
 * rather than returning an object — Safari with cookies blocked has done
 * this, and the access itself is what throws, so it cannot be checked with
 * `typeof`. In both cases a `remembered` signal degrades to a `client`
 * one: it works for the life of the tab and forgets on reload. That is the
 * one place this file guesses, and it guesses toward the program still
 * running.
 */
function storage() {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch (unavailable) {
    return null;
  }
}

function load(store, key, initial) {
  if (!store) return initial;
  const stored = store.getItem(key);
  return stored === null ? initial : parse(stored);
}

function save(store, key, value) {
  if (!store) return;
  store.setItem(key, stringify(value));
}
