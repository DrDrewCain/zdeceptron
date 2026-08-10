// `clock.js` against a scheduler this file controls.
//
// **The scheduler is fake on purpose, and it is the whole value of the
// suite.** A test that waits for a real 250 ms timer proves the timer
// fired and nothing about what happens to it afterwards; the question
// worth answering is the one a wall clock cannot ask — *does a disposed
// view stop firing* — and that needs a scheduler whose queue can be
// inspected after the dispose. So `setInterval`, `setTimeout` and
// `requestAnimationFrame` are replaced here with a queue and an
// `advance()`, and every case below is deterministic and instant.
//
// `signal.js` and `clock.js` are evaluated into the same scope by
// `zdc-runtime/tests/render.rs`, which flattens the imports away.

// --- the fake scheduler ---------------------------------------------------

const scheduled = new Map();
let nextId = 1;
let clockNow = 0;

globalThis.performance = { now: () => clockNow };

globalThis.setInterval = (fn, ms) => {
  const id = nextId++;
  scheduled.set(id, { fn, ms, at: clockNow + ms, repeat: true, kind: 'interval' });
  return id;
};
globalThis.clearInterval = (id) => scheduled.delete(id);

globalThis.setTimeout = (fn, ms) => {
  const id = nextId++;
  scheduled.set(id, { fn, ms, at: clockNow + ms, repeat: false, kind: 'timeout' });
  return id;
};
globalThis.clearTimeout = (id) => scheduled.delete(id);

globalThis.requestAnimationFrame = (fn) => {
  const id = nextId++;
  // 16 ms is not a promise the browser makes either; what matters is that
  // it is a positive step, so `now - base` moves.
  scheduled.set(id, { fn, ms: 16, at: clockNow + 16, repeat: false, kind: 'frame' });
  return id;
};
globalThis.cancelAnimationFrame = (id) => scheduled.delete(id);

/** Advance virtual time by `ms` and run everything that comes due. */
function advance(ms) {
  const target = clockNow + ms;
  // Bounded: a frame loop reschedules itself, so an unbounded drain over a
  // self-refilling queue is a hang rather than a failure.
  for (let guard = 0; guard < 1000; guard += 1) {
    let earliest = null;
    scheduled.forEach((entry, id) => {
      if (entry.at <= target && (earliest === null || entry.at < earliest.at)) {
        earliest = { id, ...entry };
      }
    });
    if (earliest === null) break;
    clockNow = earliest.at;
    if (earliest.repeat) {
      scheduled.get(earliest.id).at = clockNow + earliest.ms;
    } else {
      scheduled.delete(earliest.id);
    }
    earliest.fn(clockNow);
  }
  clockNow = target;
}

/** How many callbacks the scheduler is still holding.
 *
 * Not called `pending`: `signal.js` already has a top-level `pending` and
 * this file is evaluated into the same scope, so that name is a duplicate
 * lexical declaration rather than a shadow. */
function booked() {
  return scheduled.size;
}

function reset() {
  scheduled.clear();
  clockNow = 0;
}

// --- `every "<duration>"` -------------------------------------------------

test('an interval signal starts at zero and reads elapsed milliseconds', () => {
  reset();
  const [elapsed] = owned(() => everyMs(250));
  assert.equal(elapsed(), 0, 'before the first tick');
  advance(250);
  assert.equal(elapsed(), 250, 'after one tick');
  advance(500);
  assert.equal(elapsed(), 750, 'after three');
});

test('an interval signal reaches the bindings that read it', () => {
  reset();
  const seen = [];
  owned(() => {
    const elapsed = everyMs(100);
    effect(() => seen.push(elapsed()));
  });
  advance(300);
  assert.deepEqual(seen, [0, 100, 200, 300]);
});

// **The leak test.** A discarded view must stop firing.
test('disposing the owner stops the interval', () => {
  reset();
  const seen = [];
  const [, dispose] = owned(() => {
    const elapsed = everyMs(100);
    effect(() => seen.push(elapsed()));
  });
  advance(200);
  assert.equal(seen.length, 3, 'it ran while it was alive');
  dispose();
  assert.equal(booked(), 0, 'the scheduler is holding nothing');
  advance(1000);
  assert.equal(seen.length, 3, 'and nothing arrived after the dispose');
});

test('an interval reads time since it started, not since the page did', () => {
  reset();
  advance(1000);
  const [elapsed] = owned(() => everyMs(250));
  advance(250);
  assert.equal(elapsed(), 250, 'the origin is the declaration, not the epoch');
});

test('two clocks in different owners are disposed independently', () => {
  reset();
  const [kept] = owned(() => everyMs(100));
  const [dropped, dispose] = owned(() => everyMs(100));
  advance(100);
  assert.equal(kept(), 100);
  assert.equal(dropped(), 100);
  dispose();
  advance(200);
  assert.equal(dropped(), 100, 'the disposed one stopped');
  assert.equal(kept(), 300, 'and the other did not');
  assert.equal(booked(), 1, 'exactly one timer is left');
});

// --- `every frame` --------------------------------------------------------

test('a frame signal starts at zero on its own first frame', () => {
  reset();
  // Time has already moved before the loop starts, which is the case that
  // made the base the first *frame* rather than the call: subtracting the
  // call time would start this signal at 500 rather than 0.
  advance(500);
  const [motion] = owned(() => everyFrame());
  assert.equal(motion(), 0, 'before any frame');
  advance(16);
  assert.equal(motion(), 0, 'the first frame is the origin');
  advance(16);
  assert.equal(motion(), 16, 'and the second is one frame past it');
});

test('a frame loop keeps rescheduling itself', () => {
  reset();
  const seen = [];
  owned(() => {
    const motion = everyFrame();
    effect(() => seen.push(motion()));
  });
  advance(16 * 5);
  assert.ok(seen.length >= 5, `expected at least five frames, saw ${seen.length}`);
  assert.ok(booked() > 0, 'and one is still booked');
});

// **The other leak test**, and the one with a race in it: a frame callback
// the browser has already dequeued must not book its successor after the
// dispose. `cancelAnimationFrame` alone does not cover that, because the
// frame it cancels is not the one in flight.
test('disposing the owner stops the frame loop', () => {
  reset();
  const seen = [];
  const [, dispose] = owned(() => {
    const motion = everyFrame();
    effect(() => seen.push(motion()));
  });
  advance(16 * 3);
  const before = seen.length;
  dispose();
  assert.equal(booked(), 0, 'the scheduler is holding nothing');
  advance(16 * 100);
  assert.equal(seen.length, before, 'and no frame arrived after the dispose');
});

// --- `after "<duration>"` -------------------------------------------------

test('a delay signal is false until it fires and true forever after', () => {
  reset();
  const [ready] = owned(() => afterMs(2000));
  assert.equal(ready(), false, 'before');
  advance(1999);
  assert.equal(ready(), false, 'one millisecond short');
  advance(1);
  assert.equal(ready(), true, 'on time');
  advance(10000);
  assert.equal(ready(), true, 'and it stays');
});

test('a delay fires once and leaves nothing scheduled', () => {
  reset();
  const seen = [];
  owned(() => {
    const ready = afterMs(50);
    effect(() => seen.push(ready()));
  });
  advance(500);
  assert.deepEqual(seen, [false, true]);
  assert.equal(booked(), 0, 'a one-shot clears itself by firing');
});

test('disposing before the delay elapses cancels it', () => {
  reset();
  const seen = [];
  const [, dispose] = owned(() => {
    const ready = afterMs(1000);
    effect(() => seen.push(ready()));
  });
  dispose();
  assert.equal(booked(), 0, 'the timeout was cleared');
  advance(5000);
  assert.deepEqual(seen, [false], 'and it never became true');
});

// --- what the emitter relies on -------------------------------------------

test('every clock constructor returns a read function and nothing else', () => {
  reset();
  owned(() => {
    for (const made of [everyMs(100), everyFrame(), afterMs(100)]) {
      assert.equal(typeof made, 'function', 'a clock is read like any signal');
      // No setter is handed out anywhere: there is no way for a program to
      // write one of these cells, which is what the checker refuses in
      // source and this is the runtime half of.
      assert.equal(made.length, 0, 'and it takes no argument');
    }
  });
});
