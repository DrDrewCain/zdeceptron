// The lifecycle of a `foreign … gives view` — spec §14E.1, §14E.3.
//
// **Its own module, and that is a size decision rather than a tidiness
// one.** A DOM-owning foreign is the one construct here that a program
// can go its whole life without writing, and §16.3.1 promises a bundle
// ships nothing it does not use. Left in `dom.js` these bytes were
// downloaded by every page ever served, including one with no FFI in it,
// which is a fixed cost paid for an optional feature. `Bundle::runtime`
// already computes a transitive import closure — `rpc.js`, `store.js` and
// `wire.js` are linked only when the split finds a crossing or a durable
// key — so this is that existing mechanism applied once more, and not a
// new exemption from the size gate. `zdc-bench` charges this file to the
// programs that link it and to no others, and a test pins both halves of
// that: a null program must not reach it, and a program with a `gives
// view` foreign must.
//
// Nothing here touches the DOM. The node is handed in — the template
// already carries it — so this module needs the reactivity core and
// nothing else, which is why it does not import `dom.js` and why linking
// it costs a program `signal.js` it already had.
//
// **The contract is checked here because here is the only place that can
// see it (#239).** `mount(node, props) -> { update(props), destroy() }` is
// a shape no type in the language describes, so `from "three" as "Scene"`
// compiles — a `foreign` declaration gives the compiler nothing to check
// it against — and used to fail on the first render with an engine
// `TypeError` raised inside this file, naming a local the reader never
// wrote and no part of the declaration that caused it.
//
// It is not cheap: the check nearly trebles this file, almost all of it
// the refusals' own prose, and the module is downloaded whole by every
// program that writes one of these. BENCHMARKS.md charges it there and
// records what it did to the margin, because a size argument that quietly
// stops applying to the file it was made about is worse than the bytes.
// What it buys is the sentence that turns a trace through a runtime into
// the name of a declaration to open.

import { effect, onCleanup } from './signal.js';

/** The contract, spelled once and quoted verbatim in every refusal. */
const CONTRACT = 'mount(node, props) -> { update(props), destroy() }';

/** The claim both refusals of the imported binding open with. */
const NOT_A_MOUNT = 'gives a view, so what its `as` clause names must be a mount function; ';

/**
 * Hand an element to a `foreign … gives view` (§14E.1, §14E.3).
 *
 * `node` is a `<div>` the template already carries, so a foreign is a
 * static-markup hole bound like an attribute rather than an anchor pair
 * like `each`, keeping it inside §16.2 R2's cloning model. `props` is a
 * thunk giving a plain object, one property per `takes` argument in
 * order, read inside an effect. `declared` is the declaration's own name
 * in the program, carried here for no reason but the refusals: nothing
 * else in this file reads it, and without it a breach of the contract can
 * only be reported against a runtime the reader did not write.
 *
 * Reactivity is `update`, never re-invocation: re-running `create` would
 * rebuild whatever the module owns — a WebGL context, an animation — on
 * every write, the failure this form prevents. Nothing crosses back, and
 * the handle's *types* are still asserted rather than verified (§14E.4) —
 * what is checked here is that there is a handle with the two methods,
 * which is the part that has an answer at mount.
 */
export function foreign(node, create, props, declared) {
  let handle = null;
  let disposed = false;

  effect(() => {
    // Above the guard, so the edge exists on a run that bails.
    const next = props();
    // Disposal cannot retract a run the flush has already queued:
    // `clearSources` unsubscribes for the future, and a pending run is
    // still in the drain list. Without this, removing an `each` row in
    // the same batch as a write that row read calls `update` on a
    // destroyed handle — a fault with no visible symptom.
    if (disposed) return;
    if (handle === null) {
      handle = mounted(create, node, next, declared);
      return;
    }
    handle.update(next);
  });

  // `owned` disposes **last-registered-first**, so this runs *before* the
  // effect above is unsubscribed — `destroy()` lands while the binding is
  // still live. That is the opposite of what this form was first written
  // against, when `owned` disposed in registration order, and it is why
  // the ordering is not what makes it safe: the `disposed` flag is. Set
  // it before `destroy()`, and a run the flush had already queued finds
  // the guard rather than a destroyed handle, whichever order the two
  // cleanups happen to run in.
  //
  // Stated because the alternative is a comment that is true only of a
  // disposal order nothing here enforces.
  onCleanup(() => {
    disposed = true;
    if (handle !== null) handle.destroy();
  });
}

/**
 * Call `create` and return its handle, or refuse in the declaration's name.
 *
 * Checked at mount and nowhere afterwards. A handle that answered once
 * cannot stop answering — the module would have to replace its own return
 * value, which it no longer holds — so re-checking on every write would
 * charge every signal write for a mistake that can only be made once.
 */
function mounted(create, node, props, declared) {
  if (typeof create !== 'function') {
    refuse(
      declared,
      NOT_A_MOUNT + 'this one is ' + describe(create) + '.',
      'Point it at an export of that shape, or at a module of your own that wraps the library'
    );
  }
  // A class passes `typeof create === 'function'`, and a class is what
  // every visual library exports — three.js's `Scene`, chart.js's `Chart`,
  // maplibre's `Map` — so this is the case the check exists for rather
  // than an edge of it. Calling one raises `Class constructor … cannot be
  // invoked without 'new'`, from this file, about a name the reader never
  // wrote. That report is what #239 was filed about.
  if (isClass(create)) {
    refuse(
      declared,
      NOT_A_MOUNT + 'this one is a class, and a class cannot be called without `new`.',
      "A library's class is not a mount function: give `" +
        declared +
        '` a module of your own that constructs it, hands it the node, and returns the handle'
    );
  }

  const handle = create(node, props);
  if (!conforms(handle)) {
    refuse(
      declared,
      'gives a view, so mounting it must return a handle; this ' + returned(handle) + '.',
      '`update` is how a write reaches the module — re-invoking mount would rebuild whatever it ' +
        'owns, a WebGL context or an animation in flight — and `destroy` is how the module gives ' +
        'back a frame loop or a context when the node goes. Return both'
    );
  }
  return handle;
}

/**
 * Throw the one shape of refusal this file has: claim, contract, repair.
 *
 * One function rather than three literals so that the declaration is named
 * the same way every time and the spec reference cannot drift between
 * them — a reader who has seen one of these has seen all three.
 *
 * `repair` carries no closing full stop: this adds one after the spec
 * reference, which is the sentence's real end.
 */
function refuse(declared, claim, repair) {
  throw new Error(
    '`' + declared + '` ' + claim + ' The contract is ' + CONTRACT + '. ' + repair + ' (spec §14E.1).'
  );
}

/** Whether a handle is one: an object carrying both halves of the contract. */
function conforms(handle) {
  return (
    handle !== null &&
    typeof handle === 'object' &&
    typeof handle.update === 'function' &&
    typeof handle.destroy === 'function'
  );
}

/** What was imported, as a noun phrase: `a number`, `an object`, `null`. */
function describe(value) {
  if (value === null) return 'null';
  const what = typeof value;
  if (what === 'undefined') return 'undefined';
  return ('aeiou'.includes(what[0]) ? 'an ' : 'a ') + what;
}

/** What mounting produced, as the tail of "this …". */
function returned(handle) {
  if (handle === null || typeof handle !== 'object') return `returned ${describe(handle)}`;
  const missing =
    typeof handle.update === 'function'
      ? 'no `destroy`'
      : typeof handle.destroy === 'function'
        ? 'no `update`'
        : 'neither `update` nor `destroy`';
  return `returned an object with ${missing}`;
}

/**
 * Whether `fn` is a class rather than an ordinary function.
 *
 * ECMAScript's own distinction rather than a guess at source text: a class
 * constructor's `prototype` is non-writable, an ordinary function's is
 * writable, and a method or arrow has none at all. Reading the descriptor
 * separates the three without `Function.prototype.toString`, which a
 * minifier, a bound function and a native class can each make lie.
 *
 * A class transpiled down to a plain function — what a bundler targeting
 * ES5 emits — is not detectable here and does not need to be: it is
 * callable, so it is called, and it is the handle check that refuses it.
 */
function isClass(fn) {
  const prototype = Object.getOwnPropertyDescriptor(fn, 'prototype');
  return prototype !== undefined && prototype.writable === false;
}
