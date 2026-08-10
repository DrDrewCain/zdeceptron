// `media "…"` — a CSS media query the browser keeps answering.
//
// `matchMedia(q).matches` is a boolean read at one instant. The whole
// value of making this a language construct rather than a `foreign` is
// that a read at one instant is the wrong thing and is very easy to write:
// the survey of the site this was built for found eight `matchMedia` call
// sites, and six of them read `.matches` once at mount and never learned
// that the answer had changed. A visitor who turns on Reduce Motion while
// the page is open keeps the animation.
//
// So this returns a *signal*. The subscription is installed once per
// distinct query — the emitter hoists one cell per query literal — and
// every read of it anywhere in the program is a read of that one cell.
//
// # Its own module
//
// §16.3.1, as for `remembered.js`, `list.js`, `foreign.js` and
// `markup.js`: a program that asks the browser nothing must not ship a
// subscription it never installs. It imports `signal.js` and nothing else,
// so it never drags in `dom.js`.

import { signal } from './signal.js';

/**
 * Whether the browser matches `query`, as a signal.
 *
 * `false` where there is no `matchMedia` — the DOM shim the compiler's own
 * tests render against, and any host that is not a browser. That is the
 * right answer rather than a safe one: `prefers-reduced-motion: reduce`
 * and `prefers-color-scheme: dark` are both queries whose unmatched
 * reading is the ordinary case, and a media query nobody can evaluate has
 * not matched.
 */
export function mediaMatch(query) {
  if (typeof matchMedia !== 'function') return signal(false)[0];

  const list = matchMedia(query);
  const [read, write] = signal(list.matches);
  // `addEventListener` on a `MediaQueryList` is the modern spelling and
  // `addListener` the one Safari carried alone until 14. The fallback is
  // two lines and its absence is a silent staleness on those browsers,
  // which is the exact failure this file exists to remove.
  if (typeof list.addEventListener === 'function') {
    list.addEventListener('change', (event) => write(event.matches));
  } else if (typeof list.addListener === 'function') {
    list.addListener((event) => write(event.matches));
  }
  return read;
}
