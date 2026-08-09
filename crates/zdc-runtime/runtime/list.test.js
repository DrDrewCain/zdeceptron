// Tests for keyed list reconciliation. Run: `cargo test -p zdc-runtime`
//
// `runtime/list.js`'s own suite, for the reason `elements.test.js` is
// `elements.js`'s: each suite names the module it is about, and `boa`
// aborts the *process* with a Rust-level `BorrowMutError` inside its own
// `Set` builtin once a context's total allocation crosses a threshold —
// the defect BENCHMARKS.md records as making signal fan-out unmeasurable
// here. `dom.test.js` plus a reconciler suite sat on it, deterministically.
//
// `document`, `html`, `test` and `assert` come from the harness; `signal`,
// `el`, `text`, `template`, `bindText` and `each` come from the runtime
// evaluated in the same scope.

// --- the minimal move set (spec §16.10, issue #207) -----------------------
//
// The reconciler moves only the rows a longest increasing subsequence
// leaves out, which is the fewest moves that can produce the new order.
// How many moves that is, at three sizes and in four shapes, is measured
// in `crates/zdc-bench`. What is here is the half a count cannot check:
// that every one of those shapes still ends with the right list, and with
// the rows that did not have to move still being the same nodes.
//
// A reconciler bug is a silent wrong-DOM bug, so these are written as
// order assertions over shapes chosen to break a move-minimising pass in
// the ways such a pass breaks: a permutation that is nearly sorted, one
// that is not sorted at all, and permutations mixed with an insertion and
// a deletion so that the "where does this row sit now" bookkeeping has to
// survive rows arriving and leaving in the same update.

// A shorthand: a keyed list of single-character rows, and its order back.
function keyedRegion(initial) {
  const [items, setItems] = signal(initial.map((k) => ({ k })));
  const host = el('div', {}, []);
  host.appendChild(
    each(
      items,
      (item) => item.k,
      (get) => el('i', {}, [text(() => get().k)])
    )
  );
  return {
    host,
    set: (keys) => setItems(keys.map((k) => ({ k }))),
    order: () => host.childNodes.filter((n) => n.kind === 'element').map((n) => html(n).slice(3, -4)).join(''),
    idOf: (k) =>
      host.childNodes.find((n) => n.kind === 'element' && html(n) === `<i>${k}</i>`).__id,
  };
}

test('a swap of two rows moves only those two and keeps the rest', () => {
  const region = keyedRegion(['a', 'b', 'c', 'd', 'e', 'f']);
  const untouched = ['c', 'd', 'e'].map(region.idOf);

  region.set(['a', 'e', 'c', 'd', 'b', 'f']);
  assert.equal(region.order(), 'aecdbf');
  // `c` and `d` were between the two rows that changed places and are
  // still in order relative to each other, so a minimal move set does not
  // touch them. Node identity is how that is visible from here: a rebuilt
  // row would be a different node.
  assert.equal(region.idOf('c'), untouched[0], 'c must not be rebuilt');
  assert.equal(region.idOf('d'), untouched[1], 'd must not be rebuilt');
  assert.equal(region.idOf('e'), untouched[2], 'a moved row is moved, not rebuilt');
});

test('a full reversal ends up reversed', () => {
  const region = keyedRegion(['a', 'b', 'c', 'd', 'e']);
  const before = ['a', 'b', 'c', 'd', 'e'].map(region.idOf);

  region.set(['e', 'd', 'c', 'b', 'a']);
  assert.equal(region.order(), 'edcba');
  // The worst case for the move count is still the best case for identity:
  // every row is the node it was.
  assert.equal(region.idOf('a'), before[0], 'a reversal moves rows, it does not rebuild them');
  assert.equal(region.idOf('e'), before[4]);
});

test('a reorder with rows removed keeps the survivors in the new order', () => {
  const region = keyedRegion(['a', 'b', 'c', 'd', 'e', 'f']);
  const kept = region.idOf('e');

  region.set(['e', 'a', 'c']);
  assert.equal(region.order(), 'eac');
  assert.equal(region.idOf('e'), kept, 'a survivor is moved, not rebuilt');
});

test('a reorder with rows added places the new rows among the old', () => {
  const region = keyedRegion(['a', 'b', 'c']);
  const kept = region.idOf('b');

  region.set(['x', 'c', 'y', 'b', 'z', 'a']);
  assert.equal(region.order(), 'xcybza');
  assert.equal(region.idOf('b'), kept, 'an existing row is not rebuilt by an insertion');
});

test('a reorder that removes, adds and permutes in one update', () => {
  const region = keyedRegion(['a', 'b', 'c', 'd', 'e', 'f', 'g']);
  const kept = ['b', 'f'].map(region.idOf);

  region.set(['f', 'x', 'b', 'g', 'y', 'd']);
  assert.equal(region.order(), 'fxbgyd');
  assert.equal(region.idOf('b'), kept[0]);
  assert.equal(region.idOf('f'), kept[1]);

  // And again from the order it just produced, because the reconciler
  // records where a row sits from its own previous placement: an update
  // applied to a list that was itself reordered is where an off-by-one in
  // that bookkeeping shows up and a single update never would.
  region.set(['d', 'y', 'g', 'b', 'x', 'f']);
  assert.equal(region.order(), 'dygbxf');
  region.set(['b', 'd', 'f', 'g', 'x', 'y']);
  assert.equal(region.order(), 'bdfgxy');
});

test('a rotation by one is still a rotation after doing it five times', () => {
  const region = keyedRegion(['a', 'b', 'c', 'd', 'e']);
  const before = ['a', 'b', 'c', 'd', 'e'].map(region.idOf);
  let order = ['a', 'b', 'c', 'd', 'e'];
  for (let i = 0; i < 5; i += 1) {
    order = [order[order.length - 1], ...order.slice(0, order.length - 1)];
    region.set(order);
    assert.equal(region.order(), order.join(''), 'rotation ' + (i + 1));
  }
  // Five rotations of five rows is the identity permutation, and every
  // row is the node it started as.
  assert.equal(region.order(), 'abcde');
  assert.equal(region.idOf('a'), before[0]);
  assert.equal(region.idOf('c'), before[2]);
});

test('a multi-root row that does not move is not reinserted', () => {
  const rows = (keys) => keys.map((k) => ({ k }));
  const [items, setItems] = signal(rows(['a', 'b', 'c']));
  const host = el('div', {}, []);
  host.appendChild(
    each(
      items,
      (item) => item.k,
      (get) => {
        const row = template('<i> </i><b> </b>')();
        bindText(row.firstChild.firstChild, () => get().k);
        bindText(row.lastChild.firstChild, () => get().k);
        return row;
      }
    )
  );
  assert.equal(html(host), '<div><i>a</i><b>a</b><i>b</i><b>b</b><i>c</i><b>c</b></div>');

  // Only `a` and `c` change places; `b`'s two roots must stay adjacent and
  // stay put. A move set that reinserted `b` would still render this, so
  // the assertion that matters is the one in `crates/zdc-bench` — this one
  // says the roots did not come apart.
  setItems(rows(['c', 'b', 'a']));
  assert.equal(html(host), '<div><i>c</i><b>c</b><i>b</i><b>b</b><i>a</i><b>a</b></div>');
});
