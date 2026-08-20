// `media.js` against a `matchMedia` this file controls.
//
// **This module was the one entry in `UNREACHED` with no answer to give.**
// The list exists because "which modules are covered" is a question nobody
// had asked mechanically, and asking it turned up a shipped module that
// nothing anywhere evaluated — not in this engine and not in the browser
// job. What was checked was that a bundle *links* it, which is a claim
// about the emitter.
//
// The host is faked for the reason `clock.test.js` fakes a scheduler: the
// questions worth asking are the ones a real browser cannot be made to
// answer on demand. Does a reader who turns Reduce Motion *on while the
// page is open* see the change? That is the whole reason this is a signal
// rather than a `foreign` reading `.matches` once — the survey behind it
// found six of eight call sites reading once at mount and never learning
// the answer had changed.
//
// `signal.js` and `media.js` are evaluated into the same scope by
// `zdc-runtime/tests/render.rs`, which flattens the imports away.

// --- the fake host --------------------------------------------------------

// One list per query, so a case can flip the answer and fire.
const lists = new Map();

function installMatchMedia({ modern = true } = {}) {
  lists.clear();
  globalThis.matchMedia = (query) => {
    let list = lists.get(query);
    if (list === undefined) {
      list = {
        media: query,
        matches: false,
        listeners: [],
        // A `MediaQueryList` carries both spellings in the wild: Safari
        // shipped only `addListener` until 14. The module tries the modern
        // one first, so a case that wants the fallback withholds it.
        addEventListener: modern
          ? (name, fn) => {
              if (name === 'change') list.listeners.push(fn);
            }
          : undefined,
        addListener: modern ? undefined : (fn) => list.listeners.push(fn),
      };
      lists.set(query, list);
    }
    return list;
  };
}

// What the browser does when the reader changes a preference.
function flip(query, matches) {
  const list = lists.get(query);
  list.matches = matches;
  list.listeners.forEach((fn) => fn({ matches }));
}

function withoutMatchMedia() {
  delete globalThis.matchMedia;
}

// --- a host that cannot answer --------------------------------------------

test('a host with no matchMedia reads as unmatched', () => {
  withoutMatchMedia();
  const dark = mediaMatch('(prefers-color-scheme: dark)');
  // The build host is this case, and `prerender.rs` paints what it returns.
  assert.equal(dark(), false, 'a query nobody can evaluate has not matched');
});

test('and it is a signal there too, not a bare value', () => {
  withoutMatchMedia();
  const dark = mediaMatch('(prefers-color-scheme: dark)');
  assert.equal(typeof dark, 'function', 'every reader reads it the same way');
});

// --- a host that can ------------------------------------------------------

test('a query the host matches reads as matched', () => {
  installMatchMedia();
  const reduce = mediaMatch('(prefers-reduced-motion: reduce)');
  assert.equal(reduce(), false, 'the fake starts unmatched');
  flip('(prefers-reduced-motion: reduce)', true);
  assert.equal(reduce(), true, 'the reader turned it on and the signal knows');
});

test('the reader can turn it off again', () => {
  installMatchMedia();
  const reduce = mediaMatch('(prefers-reduced-motion: reduce)');
  flip('(prefers-reduced-motion: reduce)', true);
  flip('(prefers-reduced-motion: reduce)', false);
  assert.equal(reduce(), false, 'the subscription is not one-shot');
});

test('a starting value the host already holds is read at once', () => {
  installMatchMedia();
  // The list exists and matches before anything subscribes to it.
  matchMedia('(min-width: 40em)');
  flip('(min-width: 40em)', true);
  const wide = mediaMatch('(min-width: 40em)');
  assert.equal(wide(), true, 'the first read is the host\'s answer, not false');
});

test('two queries do not answer for each other', () => {
  installMatchMedia();
  const dark = mediaMatch('(prefers-color-scheme: dark)');
  const wide = mediaMatch('(min-width: 40em)');
  flip('(min-width: 40em)', true);
  assert.equal(wide(), true, 'the query that changed changed');
  assert.equal(dark(), false, 'and the one that did not, did not');
});

// --- the spelling Safari shipped alone until 14 ---------------------------

test('a list with only addListener is still subscribed to', () => {
  installMatchMedia({ modern: false });
  const reduce = mediaMatch('(prefers-reduced-motion: reduce)');
  flip('(prefers-reduced-motion: reduce)', true);
  // Without the fallback this reads `false` for ever on those browsers,
  // which is the exact staleness this module exists to remove.
  assert.equal(reduce(), true, 'the older spelling is subscribed to as well');
});
