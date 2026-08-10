// Tests for what an emitted program does when a handler throws (#139).
//
// A suite of its own rather than more cases in `dom.test.js`, for the
// reason `elements.test.js` is a suite of its own: `boa` panics inside its
// own `Set` builtin once one context's total allocation crosses a
// threshold, and `dom.test.js` is already at it. Splitting by subject is
// also the honest division — this file is about one decision.
//
// `document`, `html`, `reported`, `test` and `assert` come from the
// harness; `signal`, `effect`, `el`, `text` and `on` come from the runtime
// evaluated in the same scope.

// --- a handler that throws (#139) -----------------------------------------
//
// The decision is that a throwing handler is contained to itself and
// reported, that the writes it made before throwing stand, and that every
// other binding on the page goes on working. Each of those is a separate
// case below, because each could break without the others noticing.

test('a handler that throws does not take the page with it', () => {
  reported.length = 0;
  const [count, setCount] = signal(0);
  const shown = el('span', {}, [text(count)]);
  const bad = el('button', {}, []);
  const good = el('button', {}, []);
  on(bad, 'click', () => {
    throw new Error('the handler failed');
  });
  on(good, 'click', () => setCount(count() + 1));

  bad.fire('click');
  assert.equal(reported.length, 1, 'the failure was reported once');
  assert.equal(String(reported[0].message), 'the handler failed', 'the original error');

  // The page is still live: a different handler still runs, and the
  // binding that reads what it wrote still updates.
  good.fire('click');
  assert.equal(html(shown), '<span>1</span>', 'the rest of the page still works');
  assert.equal(reported.length, 1, 'a working handler reports nothing');
});

test('the writes a handler made before it threw stand', () => {
  reported.length = 0;
  const [name, setName] = signal('before');
  const shown = el('span', {}, [text(name)]);
  const node = el('button', {}, []);
  on(node, 'click', () => {
    setName('after');
    throw new Error('half way');
  });

  node.fire('click');
  assert.equal(reported.length, 1, 'reported');
  // Not rolled back: the handler is not a transaction over the graph, and
  // the binding shows what it wrote. `docs/reference.md` §10 argues why.
  assert.equal(html(shown), '<span>after</span>', 'the write before the throw stands');
});

test('a binding that throws during a handler is contained the same way', () => {
  reported.length = 0;
  const [n, setN] = signal(0);
  const [other, setOther] = signal('a');
  const survivor = el('span', {}, [text(other)]);
  // A binding that throws once the signal it reads passes 0. `flush`
  // already keeps one failing computation from stopping the drain; what
  // this pins is that the failure it re-raises lands in the handler's
  // containment rather than escaping the listener.
  effect(() => {
    if (n() > 0) throw new Error('a binding failed');
  });
  const node = el('button', {}, []);
  on(node, 'click', () => {
    setN(1);
    setOther('b');
  });

  node.fire('click');
  assert.equal(reported.length, 1, 'the binding failure was reported');
  assert.equal(html(survivor), '<span>b</span>', 'the other binding in the same batch ran');
});

test('a handler attached through el is contained too', () => {
  reported.length = 0;
  // `el` routes to `on`, so there is one decision and not two. Without
  // this, the element library's handlers would be the uncontained half.
  const node = el('button', {
    onclick: () => {
      throw new Error('through el');
    },
  }, []);
  node.fire('click');
  assert.equal(reported.length, 1, 'el must not have its own listener');
  assert.equal(String(reported[0].message), 'through el');
});
