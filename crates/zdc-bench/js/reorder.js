// What a reorder costs, in moves — spec §16.10, issue #207.
//
// The main workload reorders once, at one size, in one shape: it swaps the
// second and the second-to-last of a thousand rows. That is enough to show
// that identity keying *had* a reordering cost and not enough to say what
// the cost is a function of, which is what a claim about a reconciler has
// to be. This file varies the size and the shape and counts one number:
//
//   moves — `insertBefore` calls made from JavaScript into the DOM.
//
// Every row here has exactly one root, so a move is one call and the count
// is the move set's size rather than a proxy for it.
//
// **Two arms, and the second is the algorithm this replaced.** `cursor` is
// `eachInto`'s previous placement pass, copied here unchanged: a single
// left-to-right walk that reinserts every row it finds out of place. It is
// present for the reason the `direct` arm is present in `benchmark.js` —
// so the design that was rejected is measured rather than remembered — and
// because a before-and-after with only an after is an assertion. The two
// arms must end each shape with the same DOM; the driver digests it and
// the Rust side fails the build if they differ.

const REORDER = [];

/** The rows every arm is handed, keyed by `id`. */
function rowsOf(n) {
  const out = [];
  for (let i = 0; i < n; i += 1) out.push({ id: i });
  return out;
}

/** One row: a single element, so one move is one `insertBefore`. */
function reorderRow(item) {
  const node = document.createElement('i');
  const label = document.createTextNode('');
  node.appendChild(label);
  bindText(label, () => String(item().id));
  return node;
}

// --- arm: the placement pass `eachInto` used before the LIS reconciler ---
//
// Passes 1 and 2 are the current ones; only pass 3 differs, and it differs
// exactly as it did before this change. Keeping the whole function rather
// than a diff is deliberate: a copy that shared code with the current one
// would stop being the old algorithm the moment the current one changed.

function eachIntoCursor(start, end, listGetter, keyOf, render) {
  let mounted = new Map();
  onCleanup(() => mounted.forEach((entry) => entry.dispose()));

  effect(() => {
    const items = [...(listGetter() ?? [])];
    const parent = end.parentNode;

    batch(() => {
      const keys = [];
      const live = new Set();
      for (const item of items) {
        const key = keyOf(item, keys.length);
        if (live.has(key)) {
          throw new Error('Duplicate key in a list. Keys must be unique.');
        }
        live.add(key);
        keys.push(key);
      }
      for (const [key, entry] of mounted) {
        if (!live.has(key)) {
          for (const node of entry.nodes) node.remove();
          entry.dispose();
          mounted.delete(key);
        }
      }

      const next = new Map();
      let cursor = start.nextSibling;
      for (let i = 0; i < items.length; i += 1) {
        const item = items[i];
        const key = keys[i];
        let entry = mounted.get(key);
        if (entry === undefined) {
          const [get, set] = signal(item);
          const [rendered, dispose] = owned(() => render(get));
          const nodes = rendered.nodeType === 11 ? [...rendered.childNodes] : [rendered];
          entry = { nodes, set, dispose };
        } else {
          entry.set(item);
        }
        next.set(key, entry);

        if (cursor !== entry.nodes[0]) {
          for (const node of entry.nodes) parent.insertBefore(node, cursor);
        } else {
          cursor = entry.nodes[entry.nodes.length - 1].nextSibling;
        }
      }

      mounted = next;
    });
  });
}

// --- the shapes -----------------------------------------------------------
//
// Each takes the base list and returns the list the arm is handed. The
// arms are told nothing else: working out the move set is the reconciler's
// job, which is the whole question.

const SHAPES = [
  {
    // js-framework-benchmark's own swap. Everything but two rows is
    // already in order, so a minimal move set is two moves at every size.
    name: 'swap two rows',
    of: (base) => {
      const out = base.slice();
      const a = out[1];
      out[1] = out[out.length - 2];
      out[out.length - 2] = a;
      return out;
    },
  },
  {
    // The last row is dragged to the front. One row is out of order.
    name: 'move the last row to the front',
    of: (base) => [base[base.length - 1], ...base.slice(0, base.length - 1)],
  },
  {
    // A reorder, a removal and an insertion at once, which is what an
    // ordinary update looks like and what a reconciler that only handles
    // pure permutations gets wrong.
    name: 'remove one, add one, swap two',
    of: (base) => {
      const out = base.slice();
      out.splice(4, 1);
      const a = out[1];
      out[1] = out[out.length - 2];
      out[out.length - 2] = a;
      out.push({ id: -1 });
      return out;
    },
  },
  {
    // The worst case, and it is here so the win is not overstated: a
    // reversal has no increasing subsequence longer than one row, so a
    // minimal move set is n - 1 moves and there is nothing to save.
    name: 'reverse the whole list',
    of: (base) => base.slice().reverse(),
  },
];

const SIZES = [100, 1000, 5000];

// --- the driver -----------------------------------------------------------

/**
 * A digest of the rendered order.
 *
 * The row text in sequence, hashed, so the two arms can be compared for
 * having produced the same list without the comparison living in the
 * counts. A cheaper reconciler that reordered wrongly would pass a moves
 * assertion and fail here.
 */
function orderDigest(host) {
  let hash = 0x811c9dc5;
  const push = (text) => {
    for (let i = 0; i < text.length; i += 1) {
      hash ^= text.charCodeAt(i);
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
  };
  const visit = (node) => {
    if (node.kind === 'text') {
      push(node.nodeValue);
      push('|');
    }
    for (const child of node.childNodes) visit(child);
  };
  visit(host);
  return hash;
}

function runReorderArm(name, reconcile) {
  for (const size of SIZES) {
    const base = rowsOf(size);
    const [list, setList] = signal(base);
    const host = document.createElement('div');
    const region = anchors();
    const start = region.firstChild;
    const end = region.lastChild;
    host.appendChild(region);
    reconcile(start, end, list, (item) => item.id, reorderRow);

    for (const shape of SHAPES) {
      // Back to the base order first, and not measured: what is being
      // counted is the cost of one shape from a sorted list, not the cost
      // of undoing the shape before it.
      setList(base);
      resetCounts();
      setList(shape.of(base));
      const counts = snapshot();
      REORDER.push(
        'RESULT\t' +
          name +
          '\t' +
          shape.name +
          ' at N=' +
          size +
          '\t' +
          [
            'moves=' + counts.crossings.insertBefore,
            'removals=' + counts.crossings.removeChild,
            'rows=' + host.childNodes.filter((n) => n.kind === 'element').length,
            'digest=' + orderDigest(host),
          ].join(',')
      );
    }
  }
}

runReorderArm('lis', eachInto);
runReorderArm('cursor', eachIntoCursor);

REORDER.join('\n');
