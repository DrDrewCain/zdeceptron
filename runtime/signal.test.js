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
