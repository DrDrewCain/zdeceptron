// Keyed list reconciliation — `each item in list`.
//
// **Its own module, and that is a size decision rather than a tidiness
// one** — the same decision `foreign.js` and `markup.js` record. A list is
// something a program can go its whole life without writing, and §16.3.1
// promises a bundle ships nothing it does not use. Left in `dom.js` these
// bytes were downloaded by every page ever served, including one with no
// list in it, which is a fixed cost paid for an optional feature.
// `Bundle::runtime` already computes a transitive import closure, so this
// is that existing mechanism applied once more and not a new exemption
// from the size gate: a null program must not reach this file, and a
// program with an `each` must.
//
// It needs the reactivity core and one function from `dom.js` — `anchors`,
// for the unanchored `each` wrapper — so a program that links this already
// had both.

import { signal, effect, batch, owned, onCleanup } from './signal.js';
import { anchors } from './dom.js';

/** A value that may be a signal getter or a constant. */
function read(value) {
  return typeof value === 'function' ? value() : value;
}

/**
 * Keyed list rendering — `each item in list`.
 *
 * Keys are required, not optional. Without identity, reordering destroys
 * and recreates nodes, which loses focus, scroll position, and the
 * contents of any input inside a row. That is a correctness bug, not a
 * performance one, which is why `keyOf` has no default.
 *
 * `render` receives a GETTER for its item, not the item. Reusing a node
 * across an update is a decision about DOM identity only; the row's
 * content still flows through a signal, so a changed value reaches the
 * bindings that read it without rebuilding the row.
 */
export function each(listGetter, keyOf, render) {
  const fragment = anchors();
  eachInto(fragment.firstChild, fragment.lastChild, listGetter, keyOf, render);
  return fragment;
}

/**
 * The positions of a longest increasing subsequence of `from`.
 *
 * `from[i]` is where row `i` sits in the DOM *now*, or `-1` if the row is
 * new. A subsequence that is already increasing is already in the right
 * relative order, so every row in it can stay where it is; every other row
 * has to move. Taking the *longest* such subsequence is therefore the same
 * thing as making the fewest moves, which is what §16.10 schedules and
 * what the cursor walk this replaced could not do — that walk moved every
 * row it found out of place, so exchanging the second and the
 * second-to-last of a thousand rows moved 997 of them.
 *
 * Patience sorting, O(n log n): `tails[k]` is the position ending the
 * shortest increasing run of length `k + 1` found so far, `previous[i]` is
 * what precedes `i` in the run that ends there, and the answer is read
 * back along `previous` from the last element of the longest run. New rows
 * are skipped rather than assigned a position, because a row that does not
 * exist yet cannot be left where it is.
 */
function settledPositions(from) {
  const previous = new Array(from.length);
  const tails = [];
  for (let i = 0; i < from.length; i += 1) {
    if (from[i] === -1) continue;
    let low = 0;
    let high = tails.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (from[tails[middle]] < from[i]) low = middle + 1;
      else high = middle;
    }
    previous[i] = low === 0 ? -1 : tails[low - 1];
    tails[low] = i;
  }
  const settled = new Set();
  let i = tails.length === 0 ? -1 : tails[tails.length - 1];
  while (i !== -1) {
    settled.add(i);
    i = previous[i];
  }
  return settled;
}

/**
 * Keyed list rendering between two existing anchors.
 *
 * Three passes, and the order matters.
 *
 * Departed rows are retired *before* anything is placed: a node about to
 * be removed must not block the cursor, or every row after a deletion gets
 * moved. Measured at N=1000, removing one row cost 994 moves under a
 * single pass and 0 under this one.
 *
 * Placement then runs right to left over a minimal move set, rather than
 * left to right over a cursor. The two agree on what the DOM should look
 * like and disagree on how many `insertBefore` calls it takes to get
 * there: see [`settledPositions`].
 */
export function eachInto(start, end, listGetter, keyOf, render) {
  /** key -> { nodes, set, dispose } */
  let mounted = new Map();

  // Rows are built inside the effect, where no scope is current.
  onCleanup(() => mounted.forEach((entry) => entry.dispose()));

  effect(() => {
    // Spread, not the value itself: pass 2 indexes `items`, and a list a
    // program built with `append` is an iterable chain until something
    // asks it to be an array. Iterating it is what asks. Pass 1 walks the
    // whole list anyway, so this costs no order of growth.
    const items = [...(read(listGetter) ?? [])];
    const parent = end.parentNode;

    batch(() => {
      // Pass 1: key the items, refusing duplicates before anything moves
      // (this used to fire in pass 2), and retire what left the list.
      const keys = [];
      const live = new Set();
      for (const item of items) {
        const key = keyOf(item, keys.length);
        if (live.has(key)) {
          throw new Error(`Duplicate key ${JSON.stringify(key)} in a list. Keys must be unique.`);
        }
        live.add(key);
        keys.push(key);
      }
      // `forEach` rather than `for…of` — see the engine note in
      // `signal.js`, which is a crash and not a style rule. Deleting the
      // entry being visited is defined behaviour and is what this does.
      mounted.forEach((entry, key) => {
        if (live.has(key)) return;
        for (const node of entry.nodes) node.remove();
        entry.dispose();
        mounted.delete(key);
      });

      // Pass 2: create, re-supply, and record where each survivor sits.
      //
      // `mounted` now holds exactly the surviving rows, and a `Map`
      // iterates in insertion order, so walking it gives them in the order
      // pass 3 last placed them — which is DOM order. That walk is the
      // only reason this needs no per-row bookkeeping between updates.
      const was = new Map();
      mounted.forEach((_entry, key) => was.set(key, was.size));

      const next = new Map();
      const rows = new Array(items.length);
      const from = new Array(items.length);
      for (let i = 0; i < items.length; i += 1) {
        const item = items[i];
        const key = keys[i];
        let entry = mounted.get(key);
        if (entry === undefined) {
          // `render` receives a GETTER, not a value: the row outlives any
          // one version of the item, so its bindings must read through the
          // graph. Reusing a node is then only a decision about DOM
          // identity — the row's *content* still flows reactively.
          const [get, set] = signal(item);
          // Own the row's bindings so removing it unsubscribes them.
          const [rendered, dispose] = owned(() => render(get));
          // A row may legally have several roots, so an entry holds a node
          // LIST. Capture it before insertion empties the fragment.
          const nodes =
            rendered.nodeType === 11 ? [...rendered.childNodes] : [rendered];
          entry = { nodes, set, dispose };
          from[i] = -1;
        } else {
          // The key survived; the value need not have. Re-supplying it is
          // what makes an update to a row that kept its key visible.
          entry.set(item);
          from[i] = was.get(key);
        }
        rows[i] = entry;
        next.set(key, entry);
      }

      // Pass 3: place, right to left, moving only what has to move.
      //
      // Right to left because the anchor a row is inserted before is the
      // row after it, and that row is already final by the time this one
      // is considered. A settled row is skipped rather than reinserted at
      // the position it already occupies, which is where the saving is: a
      // multi-root row costs one `insertBefore` per root, so a move that
      // did not need making is not one call but as many as the row has
      // roots.
      const settled = settledPositions(from);
      let cursor = end;
      for (let i = items.length - 1; i >= 0; i -= 1) {
        const entry = rows[i];
        if (!settled.has(i)) {
          for (const node of entry.nodes) parent.insertBefore(node, cursor);
        }
        cursor = entry.nodes[0];
      }

      mounted = next;
      // $dev
      assertPlaced(start, end, keys, mounted);
      // $end
    });
  });
}
// $dev

/**
 * Assert the nodes between the anchors are this list's rows, in order.
 *
 * The reconciliation above is the one piece of this runtime whose mistakes
 * are invisible: a row placed at the wrong index still renders, still
 * updates, and still reads correctly to every test that asks a binding for
 * its value — it is simply in the wrong place, which only a person looking
 * at the page notices. That is the shape of defect this repository has
 * repeatedly found by running the emitted program and no other way.
 *
 * So the invariant the three passes exist to establish is stated here and
 * checked: the anchored region holds exactly each row's nodes, each row
 * once, in the order the list gave. It is O(rows) on top of a pass that is
 * already O(rows), which is affordable in a development build and is
 * exactly why it is not in a release one.
 *
 * It is worth more here than it was against the cursor walk this replaced.
 * A minimal move set is computed rather than swept: `settledPositions`
 * decides which rows are *not* touched, so a wrong answer from it leaves a
 * row where it was and moves nothing — which is precisely the failure no
 * move count and no binding read can see.
 */
function assertPlaced(start, end, keys, mounted) {
  const placed = [];
  for (let node = start.nextSibling; node && node !== end; node = node.nextSibling) {
    placed.push(node);
  }
  const expected = [];
  for (const key of keys) {
    for (const node of mounted.get(key).nodes) expected.push(node);
  }
  for (let i = 0; i < Math.max(placed.length, expected.length); i += 1) {
    if (placed[i] !== expected[i]) {
      throw new Error(
        `A list of ${keys.length} rows placed ${placed.length} nodes where ` +
          `${expected.length} were reconciled, first differing at ${i}. ` +
          `Reconciliation moved a row to the wrong place.`
      );
    }
  }
}
// $end

/**
 * The interim key function: identity is the slot a row occupies.
 *
 * Spec §14G.6a reconciles by identity when the element type is a record
 * declaring `unique`, and positionally otherwise. There are no `record`
 * declarations yet, so every list is positional today. When `unique`
 * lands this is the one argument at the one call site that changes.
 */
export function byPosition(item, index) {
  return index;
}
