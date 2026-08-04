// Fine-grained reactivity for ZDeceptron.
//
// Dependencies are discovered at READ time, at runtime — never from the
// shape of the source. Spec §5.5 requires this: Svelte documented that
// compile-time dependency detection silently breaks when an expression is
// extracted into a helper function, because the dependency stops being
// visible at the declaration site. Tracking reads means refactoring cannot
// break reactivity.
//
// The model is SolidJS's: a signal write marks its readers stale and
// schedules them; a read inside a running computation registers an edge.
// There is no virtual DOM and no component re-render — a write reaches
// exactly the bindings that read it.

/** The computation currently running, or null at the top level. */
let listener = null;

/** Collects disposers created inside `owned`, or null outside one. */
let owner = null;

/** Depth of the current batch. Writes flush when it returns to zero. */
let batchDepth = 0;

let flushing = false;

/** Computations marked stale during the current batch. */
const pending = new Set();

/** Register a teardown with the scope `owned` opened, if any. */
export function onCleanup(fn) {
  if (owner) owner.push(fn);
}

/**
 * A mutable value that tracks who reads it.
 *
 * Returns a [read, write] pair rather than an object so that reading is a
 * call — which is what makes the dependency edge observable.
 */
export function signal(initial) {
  let value = initial;
  const readers = new Set();

  function read() {
    if (listener) {
      readers.add(listener);
      listener.sources.add(readers);
    }
    return value;
  }

  function write(next) {
    const resolved = typeof next === 'function' ? next(value) : next;
    // Reference equality is the right test here: ZDeceptron values are
    // immutable, so a structurally-equal new object is a genuine change
    // from the language's point of view.
    if (Object.is(resolved, value)) return value;
    value = resolved;
    // One flush per write: one at a time, a diamond ran its effect on a
    // pair of values that never existed together.
    batch(() => {
      for (const reader of [...readers]) invalidate(reader);
    });
    return value;
  }

  return [read, write];
}

/**
 * A value computed from other signals, recomputed when they change.
 *
 * This is `from` in the language. It is lazy: the body does not run until
 * something reads it, and it does not re-run until a dependency changes.
 */
export function derived(compute) {
  let value;
  let stale = true;
  const readers = new Set();

  const node = {
    sources: new Set(),
    run() {
      stale = true;
      for (const reader of [...readers]) invalidate(reader);
    },
  };

  return function read() {
    if (listener) {
      readers.add(listener);
      listener.sources.add(readers);
    }
    if (stale) {
      clearSources(node);
      const previous = listener;
      listener = node;
      try {
        value = compute();
      } finally {
        listener = previous;
      }
      stale = false;
    }
    return value;
  };
}

/**
 * Run a function now, and again whenever anything it read changes.
 *
 * Every DOM binding is one of these, which is why an update touches only
 * the nodes that actually read the changed signal.
 */
export function effect(fn) {
  // `clearSources` cannot retract a run the drain has snapshotted.
  let live = true;
  const node = {
    sources: new Set(),
    run() {
      if (!live) return;
      clearSources(node);
      const previous = listener;
      const scope = owner;
      listener = node;
      owner = null;
      try {
        fn();
      } finally {
        listener = previous;
        owner = scope;
      }
    },
  };
  node.run();
  const dispose = () => {
    live = false;
    clearSources(node);
  };
  onCleanup(dispose);
  return dispose;
}

/**
 * Run `fn`, collecting every effect it creates so they can be torn down
 * together.
 *
 * Without this a removed list row stays subscribed to whatever it read
 * for the life of the page. It is not visible as wrong output — a row
 * nobody writes to simply never re-runs — which is exactly why it needs
 * an explicit mechanism rather than being noticed.
 */
export function owned(fn) {
  const previous = owner;
  const disposers = [];
  const dispose = () => {
    while (disposers.length > 0) disposers.pop()();
  };
  // Linked, so a parent reaches it: unlinked, 2000 mount/unmount cycles
  // of three rows kept all 6000 effects live.
  if (previous) previous.push(dispose);
  owner = disposers;
  try {
    return [fn(), dispose];
  } finally {
    owner = previous;
  }
}

/**
 * Apply several writes and flush once.
 *
 * An event handler is implicitly batched, so `add 1 to a` followed by
 * `set b to 2` repaints once rather than twice.
 */
export function batch(fn) {
  batchDepth += 1;
  try {
    return fn();
  } finally {
    batchDepth -= 1;
    if (batchDepth === 0) flush();
  }
}

function invalidate(node) {
  pending.add(node);
  if (batchDepth === 0) flush();
}

/** Computations one update may run before it is a cycle. */
const STEP_LIMIT = 1e5;

function flush() {
  // A re-entrant flush is the same drain: flushing from inside a running
  // computation cost a stack frame per link, and 200 chained bindings
  // exhausted the budget.
  if (flushing) return;
  flushing = true;
  let steps = 0;
  // A throwing binding must not take the rest of the drain with it.
  let failure = null;
  try {
    // Draining rather than iterating: a computation may invalidate another,
    // and that one must run in the same flush or the DOM ends up showing a
    // value that is already out of date.
    while (pending.size > 0) {
      const ready = [...pending];
      pending.clear();
      for (const node of ready) {
        if ((steps += 1) > STEP_LIMIT) {
          throw new Error(`An update ran ${STEP_LIMIT} steps without settling.`);
        }
        try {
          node.run();
        } catch (error) {
          failure ??= error;
        }
      }
    }
  } finally {
    flushing = false;
  }
  if (failure) throw failure;
}

function clearSources(node) {
  for (const readers of node.sources) readers.delete(node);
  node.sources.clear();
}
