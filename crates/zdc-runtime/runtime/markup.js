// Rendered markup, for the one value that is parsed as HTML.
//
// **Its own module, and that is a size decision rather than a tidiness
// one** — the same decision `foreign.js` records. `Prose` is the only
// element with a `Slot::Rendered`, and a program can go its whole life
// without writing one; §16.3.1 promises a bundle ships nothing it does
// not use. Left in `dom.js` these bytes were downloaded by every page
// ever served, including one with no markup in it at all, which is a
// fixed cost paid for an optional feature. `Bundle::runtime` already
// computes a transitive import closure, so this is that existing
// mechanism applied once more and not a new exemption from the size
// gate: a null program must not reach this file, and a program with a
// `Prose` must.
//
// It needs the reactivity core and nothing else — the node is handed in,
// as it is for a foreign — so linking it costs a program `signal.js` it
// already had.

import { effect } from './signal.js';

/**
 * Replace an element's content with parsed HTML.
 *
 * **This is the only function in the runtime that parses HTML, and it is
 * the only assignment to `innerHTML` anywhere in it.** Everything else a
 * program renders reaches the DOM through `nodeValue`, `setAttribute`,
 * `.value` or `.checked`, none of which parses (spec §16.3.5). Adding
 * this narrows that claim rather than dropping it, and the narrowing is
 * carried by the compiler, not by anything here:
 *
 * * The emitter calls this from one place — `Slot::Rendered`, which only
 *   `Prose` has.
 * * `Prose`'s argument must have type `Markup`, which `Text` is not and
 *   does not convert to.
 * * The one producer of a `Markup` is `build markdown`, which runs inside
 *   the compiler over a file in the project directory, and which escapes
 *   every raw HTML span and rewrites every non-http(s) URL before
 *   returning.
 *
 * So this function trusts its argument, and the reason that is sound is
 * that no user-supplied value can ever become one. It performs no
 * sanitising of its own: a sanitiser here would be a second, weaker copy
 * of a guarantee the type system already makes, and the failure mode of
 * two disagreeing checks is worse than one.
 */
export function markup(node, value) {
  node.innerHTML = value === null || value === undefined ? '' : String(value);
}

/**
 * The same, re-parsed whenever the value changes.
 */
export function bindMarkup(node, getter) {
  effect(() => {
    const value = typeof getter === 'function' ? getter() : getter;
    const next = value === null || value === undefined ? '' : String(value);
    if (node.innerHTML !== next) node.innerHTML = next;
  });
}
