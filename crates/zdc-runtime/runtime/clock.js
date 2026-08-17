// The clock: `every "250ms"`, `every frame`, `after "2s"`.
//
// **There is no callback here, and that is the point.** Every function in
// this file takes a number and returns a *read function* — the same read
// function `signal()` returns — so what the emitter writes for a clock
// declaration is one `const` and nothing else. The scheduler's callback
// exists, but it is closed over inside this file and no program can reach
// it: all it does is put a number in a cell. Everything downstream is the
// `derived` and the bindings the language already had.
//
// That is the whole answer to "how does a timer enter a dataflow graph
// without an escape hatch": it does not enter as control flow at all. It
// is a source, like a text box, and the browser is its writer.
//
// **Its own module, for the reason `list.js` and `markup.js` are.** The
// null-program size gate (`zdc-bench/tests/scaling.rs`) keeps a 2 kB
// reserve against Swift's number, and the fix when a runtime addition eats
// into it is to ship the addition only to the programs that use it rather
// than to move the ceiling. A program with no clock links nothing here.

import { onCleanup, signal } from './signal.js';

/** `every "<duration>"` — milliseconds elapsed, written every `ms`.
 *
 * The value is *elapsed time*, not a tick count, and not the wall clock.
 *
 * - Not a count, because a count answers "how many" and almost every use
 *   wants "how long": a progress bar, a countdown and a carousel are all
 *   arithmetic on a duration, and a count makes each of them divide by the
 *   interval to get back what the timer already knew.
 * - Not `Date.now()`, because a signal holding the wall clock changes
 *   every time it is written whether or not anything moved, and because
 *   `static` and `server` placements are refused anyway — so the one
 *   question a wall clock answers ("what time is it") is the prelude's
 *   `clock`, which is where it already lives.
 *
 * Measured from the same base every time rather than accumulated, so a
 * late tick does not shift every later one: `setInterval` drifts, and a
 * clock whose drift compounds is one that visibly disagrees with a second
 * clock beside it after a minute. */
/**
 * A clock that *folds*: `every "90ms" starting v to <next>`.
 *
 * `everyMs` and `everyFrame` hand back elapsed milliseconds, which lets a
 * program watch time pass and not advance anything with it. Deriving the
 * nth state from `t` only works when the state is a closed-form function
 * of it, and a cellular automaton is the standard example of one that is
 * not — so this writes `step(previous)` instead of a timestamp.
 *
 * The cell is a real `signal`, so everything downstream is unchanged: a
 * `derived` over it, a binding, a `when`. What it is not is a *source* —
 * nothing in the program may write it, exactly as with the other two —
 * and the only writer is the scheduler below.
 *
 * Pausing is the program's job and not this function's: a step that
 * gives its argument back unchanged is a paused clock, and it keeps the
 * interval running at one phase. Tearing the timer down and rebuilding it
 * would make pause and resume two timers with two phases, and a board
 * that stutters when it starts again is the bug that causes.
 */
export function steppingMs(ms, initial, step) {
  const pair = signal(initial);
  const [read, write] = pair;
  const id = setInterval(() => write(step(read())), ms);
  onCleanup(() => clearInterval(id));
  // The *pair*, not the reader. A plain clock hands back one function
  // because nothing in the program may write it; this cell's value is the
  // program's, so a handler writes it exactly as it writes any `starting`
  // signal and the scheduler is one writer among several. A board that
  // ticks still has to accept "press g to stamp a pattern".
  return pair;
}

/** The same fold, once per repaint. */
export function steppingFrame(initial, step) {
  const pair = signal(initial);
  const [read, write] = pair;
  let live = true;
  let id = requestAnimationFrame(function tick() {
    if (!live) return;
    write(step(read()));
    id = requestAnimationFrame(tick);
  });
  // `live` is what stops a callback already dequeued by the browser from
  // booking its successor after the dispose ran — the same leak
  // `everyFrame` documents, and the same fix.
  onCleanup(() => {
    live = false;
    cancelAnimationFrame(id);
  });
  return pair;
}

export function everyMs(ms) {
  const [read, write] = signal(0);
  const start = stamp();
  const id = setInterval(() => write(stamp() - start), ms);
  onCleanup(() => clearInterval(id));
  return read;
}

/** `every frame` — milliseconds elapsed, written once per repaint.
 *
 * The base is the *first* frame's timestamp rather than the time this was
 * called: `requestAnimationFrame` hands the callback a
 * `DOMHighResTimeStamp` measured from the document's time origin, so
 * subtracting a `stamp()` taken during module evaluation would start the
 * signal at however long the page took to load. Subtracting the first
 * frame starts it at zero, which is what an animation wants and what makes
 * two frame signals declared at different moments comparable.
 *
 * **A cancelled frame loop must not schedule another one.** `cancel`
 * cancels the frame already booked; the `live` flag is what stops the
 * callback that is *mid-flight* — one already dequeued by the browser —
 * from booking its successor after the dispose ran. Without it a disposed
 * loop survives roughly half the time, which is exactly the kind of leak
 * that never reproduces on the machine it is reported from. */
export function everyFrame() {
  const [read, write] = signal(0);
  let base = null;
  let live = true;
  let id = requestAnimationFrame(function step(now) {
    if (!live) return;
    if (base === null) base = now;
    write(now - base);
    id = requestAnimationFrame(step);
  });
  onCleanup(() => {
    live = false;
    cancelAnimationFrame(id);
  });
  return read;
}

/** `after "<duration>"` — `false` until `ms` have passed, then `true`.
 *
 * One-shot, so there is nothing to keep alive afterwards: the timer clears
 * itself by firing. The `onCleanup` covers the other case — a view thrown
 * away before the delay elapses, where the write would land in a cell
 * nothing reads and the browser would hold the closure until it did. */
export function afterMs(ms) {
  const [read, write] = signal(false);
  const id = setTimeout(() => write(true), ms);
  onCleanup(() => clearTimeout(id));
  return read;
}

/** A monotonic-ish millisecond reading.
 *
 * `performance.now()` where there is one, because it does not jump when
 * the system clock is corrected — an NTP step or a user changing the time
 * would otherwise make an interval signal go backwards, and every
 * subtraction downstream of it negative. `Date.now()` is the fallback and
 * not the default. */
function stamp() {
  return typeof performance !== 'undefined' && performance.now
    ? performance.now()
    : Date.now();
}
