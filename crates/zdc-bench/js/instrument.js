// Counting instrumentation, layered over the runtime's own DOM shim.
//
// It is layered rather than copied. `crates/zdc-runtime/tests/dom-shim.js`
// is the single minimal DOM in this repository, and a second copy with
// counters bolted in would drift from it — at which point the benchmark
// would be measuring a DOM the runtime tests never run against. This file
// therefore wraps what that file already defines and adds nothing to the
// semantics.
//
// Two numbers are kept per operation, and the distinction is the whole
// reason the counts mean anything:
//
//   crossings — calls made from JavaScript into the DOM. This is what a
//               browser charges for: each one is a boundary crossing with
//               argument marshalling and invalidation on the other side.
//
//   work      — nodes actually created, linked, unlinked and written,
//               including the work a single call performs internally.
//               `cloneNode(true)` is ONE crossing that allocates a whole
//               subtree; inserting a fragment is ONE crossing that links
//               every child. Reporting only crossings would flatter
//               template cloning, and reporting only work would hide the
//               reason it is faster.
//
// Nothing here is timed. A wall-clock number from an interpreter embedded
// in a Rust test is not comparable to a browser, and presenting one as if
// it were would be dishonest (spec §14A.4 asks for numbers, not for
// numbers that look like browser numbers).

const COUNTERS = [
  'createElement',
  'createTextNode',
  'createComment',
  'createFragment',
  'cloneNode',
  'insertBefore',
  'removeChild',
  'replaceChildren',
  'setAttribute',
  'removeAttribute',
  'setProperty',
  'addEventListener',
  'textWrite',
];

const REACTIVE = ['signal', 'derived', 'effect', 'effectRun'];

function zeroed(keys) {
  const out = {};
  for (const key of keys) out[key] = 0;
  return out;
}

let crossings = zeroed(COUNTERS);
let work = zeroed(COUNTERS);
let reactive = zeroed(REACTIVE);

// Depth of the DOM call currently executing. Everything a wrapper does
// inside the original implementation is work performed by one crossing,
// not a further crossing: the fragment expansion inside `insertBefore`,
// the subtree allocation inside `cloneNode`, the removals inside
// `replaceChildren`, and the implicit unlink when an attached node is
// moved.
let depth = 0;

function tick(name) {
  work[name] += 1;
  if (depth === 0) crossings[name] += 1;
}

function resetCounts() {
  crossings = zeroed(COUNTERS);
  work = zeroed(COUNTERS);
  reactive = zeroed(REACTIVE);
}

function snapshot() {
  return { crossings, work, reactive };
}

function totalCrossings(counts) {
  let total = 0;
  for (const key of COUNTERS) total += counts[key];
  return total;
}

// --- node methods ---------------------------------------------------------

function wrap(node, name, counter) {
  const raw = node[name];
  node[name] = function (...args) {
    tick(counter);
    depth += 1;
    try {
      return raw.apply(this, args);
    } finally {
      depth -= 1;
    }
  };
}

function instrumentNode(node) {
  // `appendChild` and `remove()` are deliberately NOT wrapped: the shim
  // implements them in terms of `insertBefore` and `removeChild`, so they
  // are counted there. One name per operation keeps arms that spell the
  // same operation differently comparable.
  wrap(node, 'insertBefore', 'insertBefore');
  wrap(node, 'removeChild', 'removeChild');
  wrap(node, 'replaceChildren', 'replaceChildren');
  wrap(node, 'cloneNode', 'cloneNode');

  if (node.kind === 'element') {
    wrap(node, 'setAttribute', 'setAttribute');
    wrap(node, 'removeAttribute', 'removeAttribute');
    wrap(node, 'addEventListener', 'addEventListener');
    wrap(node.style, 'setProperty', 'setProperty');
  }

  if (node.kind === 'text' || node.kind === 'comment') {
    // A text write is a property set, so counting it needs an accessor.
    // The value the node was created with is not a write.
    let value = node.nodeValue;
    Object.defineProperty(node, 'nodeValue', {
      configurable: true,
      get() {
        return value;
      },
      set(next) {
        tick('textWrite');
        value = String(next);
      },
    });
  }

  return node;
}

// --- document factories ---------------------------------------------------
//
// The shim's factories are top-level function declarations that its own
// `cloneNode` and `parseHtml` call by name, so reassigning the binding
// catches nodes created inside the DOM as well as nodes created by a
// program. `document`'s properties captured the originals, so they are
// re-pointed at the wrappers too.

const rawCreateElement = createElement;
const rawCreateTextNode = createTextNode;
const rawCreateComment = createComment;
const rawCreateDocumentFragment = createDocumentFragment;

createElement = function (tag) {
  tick('createElement');
  depth += 1;
  try {
    return instrumentNode(rawCreateElement(tag));
  } finally {
    depth -= 1;
  }
};

createTextNode = function (value) {
  tick('createTextNode');
  depth += 1;
  try {
    return instrumentNode(rawCreateTextNode(value));
  } finally {
    depth -= 1;
  }
};

createComment = function (value) {
  tick('createComment');
  depth += 1;
  try {
    return instrumentNode(rawCreateComment(value));
  } finally {
    depth -= 1;
  }
};

createDocumentFragment = function () {
  tick('createFragment');
  depth += 1;
  try {
    return instrumentNode(rawCreateDocumentFragment());
  } finally {
    depth -= 1;
  }
};

document.createElement = createElement;
document.createTextNode = createTextNode;
document.createComment = createComment;
document.createDocumentFragment = createDocumentFragment;

// --- the reactivity core --------------------------------------------------
//
// Effect COUNT is what §16.1 compares; effect RUNS are what a list
// operation actually costs, and the two diverge sharply once a list
// re-supplies every surviving row (§16.6). Both are counted.

const rawSignal = signal;
const rawDerived = derived;
const rawEffect = effect;

signal = function (initial) {
  reactive.signal += 1;
  return rawSignal(initial);
};

derived = function (compute) {
  reactive.derived += 1;
  return rawDerived(compute);
};

effect = function (fn) {
  reactive.effect += 1;
  return rawEffect(() => {
    reactive.effectRun += 1;
    return fn();
  });
};
