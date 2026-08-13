// The browser a build host does not have.
//
// # What this is for
//
// A document's first paint used to be blank. The shell is a `<div id=app>`
// and a module that fills it, so nothing is on the page until the script
// has been fetched, parsed and run — and on a slow connection that is a
// visible flash of nothing followed by the whole page arriving at once.
//
// The fix is to run the program on the build host and put its answer in
// the HTML. `dom-shim.js` already models the tree; what it does not model
// is the rest of the browser, and a `view` that reads the clock or the
// viewport touches that rest before it renders a node. These are the
// stubs that let it run — never shipped, and reachable only from
// `zdc-codegen`'s prerender pass.
//
// # Why every timer is dead
//
// **A prerender is one synchronous pass.** A `setInterval` that fired
// would bake a *later* state into the HTML than the reader's first paint
// should show — a Life board twenty generations in, a stopwatch reading
// four seconds — and hydration would then have to undo it. So a timer
// registers, hands back a handle, and never runs. The markup is the
// program's *resting* state, which is exactly what the reader should see
// before their own browser starts the clock.
//
// # Why every reading is the neutral one
//
// A media query is false, the viewport is at the top, the store is empty
// and the pointer is absent. Each is the answer a browser gives before it
// knows better, so the prerendered markup is what the client would build
// on its first tick — which is the property hydration needs, and the
// reason these are stubs with fixed answers rather than plausible ones.

let $handle = 0;

globalThis.setInterval = () => ++$handle;
globalThis.setTimeout = () => ++$handle;
globalThis.clearInterval = () => {};
globalThis.clearTimeout = () => {};
globalThis.requestAnimationFrame = () => ++$handle;
globalThis.cancelAnimationFrame = () => {};

globalThis.performance = { now: () => 0 };
globalThis.window = globalThis;
globalThis.addEventListener = () => {};
globalThis.removeEventListener = () => {};

globalThis.matchMedia = (query) => ({
  matches: false,
  media: query,
  addEventListener() {},
  removeEventListener() {},
});

// Empty, not absent. A `remembered` cell falls back to its `starting`
// value when the store has no entry, and that is the value a first-time
// reader sees — so the markup matches the commonest first paint there is.
globalThis.localStorage = {
  getItem: () => null,
  setItem() {},
  removeItem() {},
};

globalThis.location = { pathname: '/', search: '', hash: '', href: 'http://localhost/' };
globalThis.history = { pushState() {}, replaceState() {} };
globalThis.devicePixelRatio = 1;
globalThis.getComputedStyle = () => ({ getPropertyValue: () => '', color: '#000000' });
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
// No GPU on a build host, and `scene.js` checks for exactly this before
// it reaches for an adapter.
globalThis.navigator = { gpu: null };
globalThis.fetch = () => Promise.reject(new Error('a prerender makes no requests'));
