// Tests for the reactivity core. No DOM required — this is the layer the
// language's semantics actually rest on.
//
// Run: `cargo test -p zdc-runtime`
//
// These execute inside a pure-Rust JavaScript engine embedded in the
// compiler, not under Node. Verifying ZDeceptron must not require a
// JavaScript toolchain — needing Node to build the compiler would be the
// first crack in the claim that a developer installs one binary and
// nothing else (spec §7). `test` and `assert` are provided by the harness;
// `signal`, `derived`, `effect` and `batch` come from signal.js evaluated
// in the same scope.

test('a signal returns what was written', () => {
  const [get, set] = signal(1);
  assert.equal(get(), 1);
  set(2);
  assert.equal(get(), 2);
});

test('a write of an identical value does not notify', () => {
  const [get, set] = signal(1);
  let runs = 0;
  effect(() => {
    get();
    runs += 1;
  });
  assert.equal(runs, 1);
  set(1);
  assert.equal(runs, 1, 'writing the same value must not re-run readers');
  set(2);
  assert.equal(runs, 2);
});

test('derived recomputes when what it read changes', () => {
  const [count, setCount] = signal(2);
  const doubled = derived(() => count() * 2);
  assert.equal(doubled(), 4);
  setCount(5);
  assert.equal(doubled(), 10);
});

test('derived is lazy: the body does not run until it is read', () => {
  let runs = 0;
  const [, setX] = signal(1);
  derived(() => {
    runs += 1;
    return 1;
  });
  setX(2);
  assert.equal(runs, 0, 'an unread derived must never run');
});

// This is the claim that distinguishes the design from compile-time
// dependency tracking. Svelte documented that `$: area = f(width)` stops
// updating when `height` moves inside `f`, because the dependency is no
// longer visible at the declaration site. Tracking reads at runtime means
// the extraction is invisible to the reactivity.
test('extracting an expression into a helper does not break tracking', () => {
  const [width, setWidth] = signal(2);
  const [height, setHeight] = signal(3);

  // `height` is read only inside the helper — never at the declaration site.
  const multiplyByHeight = (w) => w * height();
  const area = derived(() => multiplyByHeight(width()));

  assert.equal(area(), 6);
  setHeight(10);
  assert.equal(area(), 20, 'a dependency read inside a helper must still track');
  setWidth(5);
  assert.equal(area(), 50);
});

test('reactivity is fine-grained: an unrelated write does not recompute', () => {
  const [a, setA] = signal(1);
  const [b] = signal(1);
  let aRuns = 0;
  let bRuns = 0;
  const fromA = derived(() => {
    aRuns += 1;
    return a();
  });
  const fromB = derived(() => {
    bRuns += 1;
    return b();
  });
  fromA();
  fromB();
  assert.equal(aRuns, 1);
  assert.equal(bRuns, 1);

  setA(2);
  fromA();
  fromB();
  assert.equal(aRuns, 2, 'the reader of `a` recomputes');
  assert.equal(bRuns, 1, 'the reader of `b` must not');
});

test('a dependency that stops being read stops notifying', () => {
  const [useFirst, setUseFirst] = signal(true);
  const [first, setFirst] = signal('a');
  const [second] = signal('b');
  let runs = 0;

  const chosen = derived(() => {
    runs += 1;
    return useFirst() ? first() : second();
  });

  assert.equal(chosen(), 'a');
  assert.equal(runs, 1);

  setUseFirst(false);
  assert.equal(chosen(), 'b');
  assert.equal(runs, 2);

  // `first` is no longer read, so writing it must not invalidate.
  setFirst('changed');
  chosen();
  assert.equal(runs, 2, 'a stale dependency edge must have been dropped');
});

test('a diamond dependency settles without a redundant recompute', () => {
  const [root, setRoot] = signal(1);
  const left = derived(() => root() + 1);
  const right = derived(() => root() + 2);
  let bottomRuns = 0;
  const bottom = derived(() => {
    bottomRuns += 1;
    return left() + right();
  });

  assert.equal(bottom(), 5);
  assert.equal(bottomRuns, 1);

  setRoot(10);
  assert.equal(bottom(), 23);
  assert.equal(bottomRuns, 2, 'the join must recompute once, not once per branch');
});

test('effect runs immediately and on every change', () => {
  const [get, set] = signal(1);
  const seen = [];
  effect(() => seen.push(get()));
  set(2);
  set(3);
  assert.deepEqual(seen, [1, 2, 3]);
});

test('batch flushes once for several writes', () => {
  const [a, setA] = signal(1);
  const [b, setB] = signal(1);
  let runs = 0;
  effect(() => {
    a();
    b();
    runs += 1;
  });
  assert.equal(runs, 1);

  batch(() => {
    setA(2);
    setB(2);
  });
  assert.equal(runs, 2, 'two writes in one batch must repaint once');
});

test('disposing an effect stops it', () => {
  const [get, set] = signal(1);
  let runs = 0;
  const dispose = effect(() => {
    get();
    runs += 1;
  });
  set(2);
  assert.equal(runs, 2);
  dispose();
  set(3);
  assert.equal(runs, 2, 'a disposed effect must not run again');
});

test('a write inside an effect settles in the same flush', () => {
  const [a, setA] = signal(0);
  const [b, setB] = signal(0);
  effect(() => {
    if (a() > 0) setB(a() * 2);
  });
  setA(3);
  assert.equal(b(), 6, 'a cascade must settle before the flush returns');
});

// --- lifetime -------------------------------------------------------------
//
// The tests below were written against defects the suite above did not
// reach. Each one failed on `feature/front-end`.

test('an effect disposed while it is already scheduled does not run', () => {
  const [get, set] = signal(1);
  let runs = 0;
  const dispose = effect(() => {
    get();
    runs += 1;
  });

  batch(() => {
    set(2); // marks the effect stale
    dispose(); // before the batch flushes
  });
  assert.equal(runs, 1, 'a disposed effect must not run, even when scheduled');

  set(3);
  assert.equal(runs, 1, 'and running it would have re-subscribed it for ever');
});

test('disposing an effect twice is not an error', () => {
  const [get] = signal(1);
  const dispose = effect(() => {
    get();
  });
  dispose();
  dispose();
});

test('disposing a scope disposes the scopes opened inside it', () => {
  const [get, set] = signal(1);
  let inner = 0;
  const [, disposeOuter] = owned(() => {
    owned(() => {
      effect(() => {
        get();
        inner += 1;
      });
    });
  });
  assert.equal(inner, 1);

  disposeOuter();
  set(2);
  assert.equal(inner, 1, 'a nested scope must not outlive the scope around it');
});

test('a scope disposed by its parent and by its own handle runs once', () => {
  let cleanups = 0;
  let disposeInner = null;
  const [, disposeOuter] = owned(() => {
    const [, dispose] = owned(() => {
      onCleanup(() => {
        cleanups += 1;
      });
    });
    disposeInner = dispose;
  });

  disposeInner();
  disposeOuter();
  assert.equal(cleanups, 1, 'disposal is idempotent, so both routes are safe');
});

test('an effect re-run does not register what it builds with a stale scope', () => {
  const [get, set] = signal(1);
  let cleanups = 0;
  const [, dispose] = owned(() => {
    effect(() => {
      get();
      onCleanup(() => {
        cleanups += 1;
      });
    });
  });

  set(2);
  set(3);
  dispose();
  assert.equal(cleanups, 0, 'a re-run is outside the scope that created the effect');
});

// --- the shape of one update ---------------------------------------------

test('a diamond never shows one side updated and the other stale', () => {
  const [n, setN] = signal(1);
  const left = derived(() => n() * 2);
  const right = derived(() => n() + 10);
  const seen = [];
  effect(() => {
    seen.push(left() + ':' + right());
  });
  assert.equal(seen.length, 1);

  setN(2);
  assert.equal(
    seen.join(','),
    '2:11,4:12',
    'one write is one update: 4:11 is a pair that never existed'
  );
});

test('a binding that throws does not cancel the rest of the update', () => {
  const [get, set] = signal(0);
  let healthy = 0;
  effect(() => {
    if (get() === 1) throw new Error('boom');
  });
  effect(() => {
    get();
    healthy += 1;
  });
  assert.equal(healthy, 1);

  let threw = false;
  try {
    set(1);
  } catch (e) {
    threw = true;
    assert.ok(String(e.message).includes('boom'), 'the failure still reaches the caller');
  }
  assert.ok(threw, 'the failure must not be swallowed');
  assert.equal(healthy, 2, 'the other binding was scheduled and nothing would reschedule it');
});

test('a chain of effects settles without a stack frame per link', () => {
  const depth = 400;
  const cells = [];
  for (let i = 0; i <= depth; i += 1) cells.push(signal(0));
  for (let i = 0; i < depth; i += 1) {
    const readPrevious = cells[i][0];
    const writeNext = cells[i + 1][1];
    effect(() => {
      writeNext(readPrevious());
    });
  }

  cells[0][1](7);
  assert.equal(cells[depth][0](), 7, 'the value must reach the end of the chain');
});

test('an effect that writes a signal it reads without settling is refused', () => {
  const [get, set] = signal(0);
  let threw = false;
  try {
    effect(() => {
      set(get() + 1);
    });
  } catch (e) {
    threw = true;
    assert.ok(
      String(e.message).includes('without settling'),
      'the message must name the cycle, not the symptom'
    );
  }
  assert.ok(threw, 'a cycle must fail loudly rather than freeze the tab');
});

test('an effect that writes a signal it reads and does settle is allowed', () => {
  const [get, set] = signal(0);
  effect(() => {
    const value = get();
    if (value < 5) set(value + 1);
  });
  assert.equal(get(), 5, 'a bounded cascade is a legitimate graph');
});

test('a derived whose dependency set shrinks stops tracking what it dropped', () => {
  const [useFirst, setUseFirst] = signal(true);
  const [first, setFirst] = signal('a');
  const [second, setSecond] = signal('b');
  const value = derived(() => (useFirst() ? first() : second()));
  let runs = 0;
  effect(() => {
    value();
    runs += 1;
  });

  setUseFirst(false);
  const settled = runs;
  setFirst('a2');
  assert.equal(runs, settled, 'the branch that is no longer read is no longer a dependency');
  assert.equal(value(), 'b');
  setSecond('b2');
  assert.equal(value(), 'b2', 'and the branch that is read still is one');
});
