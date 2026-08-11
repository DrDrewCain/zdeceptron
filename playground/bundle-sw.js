// The playground's file server, which is not a server.
//
// # The problem this solves
//
// `zdc build` emits a directory: `index.html`, `boot.js`, `client.js`,
// `styles.css` and `runtime/*.js`, linked by relative paths. The page
// carries a Content-Security-Policy of `default-src 'none'; script-src
// 'self'` and no inline script (#146), which is the policy a real
// deployment ships and the one worth demonstrating.
//
// That policy is also why the obvious tricks do not work. A `srcdoc`
// iframe resolves `./boot.js` against *this* page's URL, where no such
// file exists. A `blob:` URL for each file needs `script-src blob:`, which
// the emitted policy does not grant — and rewriting the policy to grant it
// would mean the playground demonstrates a document `zdc build` never
// writes.
//
// So the bundle is served, from inside the browser. A service worker sees
// requests before the network does, so `/playground/run/7/boot.js` can be
// answered from memory: a real URL, a real origin, `'self'` satisfied, and
// the emitted document verbatim. Nothing reaches `python3 -m http.server`,
// which knows nothing about any of this — the run is as server-free as the
// compile.
//
// # The generation number
//
// Each compile is served under a fresh `run/<n>/` prefix and the iframe is
// pointed at the new one. That is what stops the browser's own HTTP cache
// from answering the second compile with the first compile's `client.js`,
// without this file having to reason about cache headers, and it makes each
// run a distinct document with its own module registry — an ES module is
// evaluated once per URL, so reusing one would silently keep the first
// program's signal graph alive.

/// Every bundle this worker is currently able to serve, by generation.
///
/// Only the most recent is kept. The older ones are only reachable from an
/// iframe that has already been replaced, and keeping every compile a
/// session ever made would grow without bound in a page whose whole purpose
/// is to compile repeatedly.
const bundles = new Map();

self.addEventListener('install', () => self.skipWaiting());

// So the first compile after a hard reload is served rather than falling
// through to the network, which would 404. Without this the worker is
// installed but controls nothing until the *next* navigation.
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));

self.addEventListener('message', (event) => {
  const message = event.data;
  if (!message || message.type !== 'bundle') return;
  bundles.clear();
  bundles.set(String(message.id), new Map(Object.entries(message.files)));
  // The page waits for this before pointing the iframe anywhere. Posting
  // the bundle and navigating in the same turn is a race the page would
  // lose about as often as it won.
  event.source.postMessage({ type: 'ready', id: message.id });
});

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin) return;

  const root = new URL('run/', self.registration.scope).pathname;
  if (!url.pathname.startsWith(root)) return;

  const rest = url.pathname.slice(root.length);
  const slash = rest.indexOf('/');
  if (slash < 0) return;

  const generation = rest.slice(0, slash);
  const path = rest.slice(slash + 1);
  const bundle = bundles.get(generation);

  event.respondWith(answer(bundle, path));
});

function answer(bundle, path) {
  if (!bundle) {
    return plain(410, `run ${path} is from a compile this page has replaced`);
  }
  const source = bundle.get(path);
  if (source === undefined) {
    // Named, not blank. A bundle that links a file it does not contain is
    // exactly the bug worth seeing, and a bare 404 in the network panel is
    // the slowest way to find it.
    return plain(404, `the bundle contains no ${path}`);
  }
  return new Response(source, {
    status: 200,
    headers: {
      'content-type': type(path),
      // The generation number already makes every URL unique, so this is
      // belt and braces — but a cached `client.js` under a reused name is
      // the single most confusing failure this page could have.
      'cache-control': 'no-store',
    },
  });
}

function plain(status, text) {
  return new Response(text, {
    status,
    headers: { 'content-type': 'text/plain; charset=utf-8' },
  });
}

/// The four extensions a bundle contains, and no default beyond text.
///
/// A module served as `text/plain` is refused by the browser with a message
/// about strict MIME checking, which is a long way from "this table is
/// missing an entry".
function type(path) {
  if (path.endsWith('.html')) return 'text/html; charset=utf-8';
  if (path.endsWith('.js')) return 'text/javascript; charset=utf-8';
  if (path.endsWith('.css')) return 'text/css; charset=utf-8';
  if (path.endsWith('.json')) return 'application/json; charset=utf-8';
  return 'text/plain; charset=utf-8';
}
