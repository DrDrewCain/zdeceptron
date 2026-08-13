// `scroll` — how far down the document the reader is, as a signal.
//
// The construct exists for the same reason `media` does: the obvious
// spelling is wrong in a way that is very easy to write. `window.scrollY`
// read once at mount is a number that never changes; `on scroll` firing a
// handler is a callback running as fast as the reader can move a finger,
// which is the shape this language is built without. A scroll position is
// neither — it is a cell the browser writes, exactly as `every frame` is,
// so it is declared with `from` and read like any other signal.
//
// # Why a percentage and not a pixel offset
//
// A pixel offset means nothing without the document's height, so a program
// that wanted "how far through are we" would have to read a second
// quantity the language does not expose and divide. The fraction is the
// answer people actually want, and it is the one that cannot be computed
// from what a program can otherwise see.
//
// # One listener, coalesced to the frame
//
// A scroll fires far faster than a repaint. Writing the signal on every
// event would schedule work the compositor throws away, so the write is
// deferred to the next animation frame and collapsed: many events, one
// write, one repaint. The listener is `passive`, which tells the browser
// it will never call `preventDefault` and lets it scroll without waiting
// for this code at all.
//
// `resize` is listened to as well, because the *fraction* changes when the
// window does even though the reader has not moved.
//
// # Its own module
//
// §16.3.1, as for `media.js`: a program that never asks where the reader
// is must not ship a listener it never installs. It imports `signal.js`
// and nothing else.

import { signal } from './signal.js';

/**
 * How far the document is scrolled, 0 to 100, as a signal.
 *
 * `0` where there is no window — the DOM shim the compiler's own tests
 * render against, and any host that is not a browser. That is the right
 * answer rather than a safe one: a document nobody has scrolled is at the
 * top, and the top is zero.
 */
export function scrollFraction() {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return signal(0)[0];
  }

  const measure = () => {
    const doc = document.documentElement;
    if (!doc) return 0;
    // The scrollable distance, not the document height: a document shorter
    // than its window has nowhere to go, and dividing by its height would
    // report a fraction of a journey that cannot be taken.
    const travel = doc.scrollHeight - doc.clientHeight;
    if (!(travel > 0)) return 0;
    const y = typeof window.scrollY === 'number' ? window.scrollY : doc.scrollTop || 0;
    const fraction = (y / travel) * 100;
    // Clamped, because elastic overscroll reports past both ends and a
    // reader at the bottom should see 100 rather than 103.
    if (!(fraction > 0)) return 0;
    return fraction > 100 ? 100 : fraction;
  };

  const [read, write] = signal(measure());

  let queued = false;
  const observe = () => {
    if (queued) return;
    queued = true;
    const run = () => {
      queued = false;
      write(measure());
    };
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(run);
    else run();
  };

  window.addEventListener('scroll', observe, { passive: true });
  window.addEventListener('resize', observe, { passive: true });
  return read;
}
