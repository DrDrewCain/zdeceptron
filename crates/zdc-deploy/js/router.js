// The portable half of a deploy adapter. Byte-identical on every target.
//
// Everything in this file is ECMA-429 (Minimum common web API, 1st edition,
// December 2025): `Request`, `Response`, `ReadableStream`, `TextEncoder`,
// `URL`, `JSON`, `setInterval`. Nothing here knows which platform it is on,
// and nothing here has to.
//
// That is only half the story, and the missing half is why the file next to
// this one exists. ECMA-429 standardises the *interior* of a handler and no
// entrypoint at all: WinterTC's own `proposal-http-server-api` repository is
// empty. So the module that calls `route`, and the store it hands in, differ
// per target. Those two files are the entire per-target surface.

// The wire format this server reads and writes (#144). A third spelling
// of `runtime/wire.js`'s `VERSION`, unavoidably: this file is copied
// verbatim onto every target and imports nothing, so it cannot read the
// constant from the module that defines it. `zdc-runtime`'s
// `wire_version.rs` reads the number back out of both files and fails if
// they ever differ.
//
// No compatibility is promised across versions. A request that names a
// different one is refused with a sentence naming both, because the
// alternative — reading it anyway — is a handler running on values it
// decoded wrongly and answering plausibly.
const WIRE_VERSION = 1;
const WIRE_HEADER = 'zd-wire';
const WIRE_PARAM = 'wire';

const JSON_HEADERS = { 'content-type': 'application/json', [WIRE_HEADER]: String(WIRE_VERSION) };
const PREFIX = '/_zd/';
// The two transport paths, spelled exactly as `runtime/store.js` spells
// them. The client half is emitted by the compiler and is the same file on
// every target, so the path and the event names are its decision, not this
// router's: a deploy target that answered a name of its own would 404 the
// bundle it was generated for, and the symptom would read as "live sync
// does not work" rather than as two files disagreeing about a URL.
const LIVE = 'live';
const POLL = 'poll';

/** A JSON response. The wire format `runtime/rpc.js` expects. */
export function json(value, status = 200) {
  return new Response(JSON.stringify(value === undefined ? null : value), {
    status,
    headers: JSON_HEADERS,
  });
}

/**
 * Dispatch one request.
 *
 * Returns `null` when the path is not an endpoint, which is the entry's
 * signal to serve a static asset instead. Returning a sentinel rather than
 * taking an asset callback keeps this file free of any opinion about how a
 * platform serves files, which is the thing platforms disagree about most.
 */
export async function route(request, endpoints, store, env, config) {
  const url = new URL(request.url);
  if (!url.pathname.startsWith(PREFIX)) return null;
  const name = decodeURIComponent(url.pathname.slice(PREFIX.length));

  // The emitted handler bodies reference `$store` and `$env` as free
  // identifiers — the compiler's §8.2 injection contract, and the reason a
  // function bundle can emit zero import statements. Both are the same
  // objects for every request in an isolate, so installing them here is
  // idempotent rather than per-request state.
  globalThis.$store = store;
  globalThis.$env = env;

  if (name === LIVE) {
    const refused = refuseVersion(url.searchParams.get(WIRE_PARAM), '`live`');
    return refused || watch(request, url, store, config);
  }
  if (name === POLL) {
    const refused = refuseVersion(url.searchParams.get(WIRE_PARAM), '`poll`');
    return refused || once(url, store);
  }

  const endpoint = endpoints[name];
  if (endpoint === undefined) return json({ error: `no endpoint named ${name}` }, 404);
  if (request.method !== 'POST') return json({ error: 'an endpoint takes POST' }, 405);

  // Before the body is read, not after: a body written by a format this
  // server does not speak is not a body worth parsing, and parsing it
  // first is how "the arguments decoded to something else" becomes a 500
  // from inside a handler instead of a sentence naming the real problem.
  const refused = refuseVersion(request.headers.get(WIRE_HEADER), name);
  if (refused) return refused;

  let args;
  try {
    args = await request.json();
  } catch {
    args = null;
  }
  if (!Array.isArray(args)) {
    return json({ error: 'the body must be a JSON array of arguments' }, 400);
  }

  try {
    // Two calling conventions, because the compiler emits two: a value
    // endpoint destructures a parameter object, a command takes the
    // argument array positionally (§17.2.7).
    const value = endpoint.command
      ? await endpoint.handler(args)
      : await endpoint.handler(named(endpoint.inputs, args));
    return json(value);
  } catch (error) {
    return json({ error: message(error) }, 500);
  }
}

/**
 * A refusal when the caller names a wire format this server does not
 * speak, or names none — or `null` to carry on.
 *
 * **Absent is a mismatch, not a courtesy.** A client that sends no
 * version was built before the format had one, which makes it a different
 * format by definition; accepting it would be the exact silent decode
 * #144 exists to close, and would do it in the one case that is
 * guaranteed to happen — the first deploy after this change.
 *
 * 400 rather than 409 or 426: the request is one this server cannot read,
 * which is what 400 says. The status is not what the browser acts on
 * anyway — `runtime/rpc.js` turns any non-2xx into `Failed` and renders
 * the sentence below, which is the thing a person actually sees.
 */
function refuseVersion(named, what) {
  if (named === String(WIRE_VERSION)) return null;
  return json(
    {
      error:
        `${what} was called in wire format ${named === null ? 'none' : named} and this server ` +
        `reads ${WIRE_VERSION}. The page was built by a different compiler; reload it.`,
    },
    400
  );
}

/** The positional wire arguments as the parameter object a handler destructures. */
function named(inputs, args) {
  const out = {};
  for (let index = 0; index < inputs.length; index += 1) out[inputs[index]] = args[index];
  return out;
}

function message(error) {
  return String(error && error.message ? error.message : error);
}

/**
 * Live sync: `text/event-stream` from `Response` + `ReadableStream` and
 * nothing else. This is the part of the portability claim that really does
 * hold — producing an SSE body needs no platform API on any target.
 *
 * A store that can push (`store.watch`) is used as a push channel; one that
 * cannot is polled. The client sees the same protocol either way, so the
 * difference between Cloudflare, which never has to disconnect, and Lambda,
 * which bills every second of the stream and does not notice a client
 * leaving, is a reconnect frequency rather than a second code path.
 */
export function watch(request, url, store, config) {
  const keys = watchedKeys(url);
  if (keys.length === 0) return json({ error: '`live` needs `?keys=`' }, 400);

  const encoder = new TextEncoder();
  const started = Date.now();
  let latest = started;
  let open = true;
  let heartbeat = null;
  let stop = null;

  const stream = new ReadableStream({
    async start(controller) {
      const send = (text) => {
        if (!open) return;
        try {
          controller.enqueue(encoder.encode(text));
        } catch {
          open = false;
        }
      };

      const close = (reason) => {
        if (!open) return;
        send(`event: close\ndata: ${JSON.stringify({ reason })}\n\n`);
        open = false;
        if (heartbeat !== null) clearInterval(heartbeat);
        if (typeof stop === 'function') stop();
        try {
          controller.close();
        } catch {
          // Already closed by the platform tearing the request down.
        }
      };

      // `update` and the `seq` inside `data:` are `runtime/store.js`'s
      // wire format, not this file's: `receive` dispatches on the event
      // name and reads the cursor out of the payload, falling back to
      // `Last-Event-ID`. Both are sent so either path resumes.
      const emit = (key, value) => {
        latest = Date.now();
        const payload = JSON.stringify({ key, value, seq: latest });
        send(`id: ${latest}\nevent: update\ndata: ${payload}\n\n`);
      };

      // A reconnect resumes by resynchronising, not by replaying. None of
      // the four stores underneath keeps a change log, so honouring
      // `Last-Event-ID` as a cursor would be a promise this cannot keep;
      // sending the current value of every watched key is one it can — and
      // because those go out as `update` events carrying the value, the
      // client needs no round trip and no `resync`.
      try {
        for (const key of keys) emit(key, await store.get(key));
      } catch (error) {
        send(`event: error\ndata: ${JSON.stringify({ error: message(error) })}\n\n`);
      }

      if (typeof store.watch === 'function') {
        stop = await store.watch(keys, emit);
      } else {
        stop = poll(store, keys, emit, config.pollSeconds);
      }

      // One timer does heartbeat, idle timeout and the duration ceiling,
      // because on a platform billed by wall clock every extra timer is a
      // reason to stay awake.
      heartbeat = setInterval(() => {
        const now = Date.now();
        if (config.maxStreamSeconds > 0 && now - started >= config.maxStreamSeconds * 1000) {
          close('max-duration');
          return;
        }
        if (config.idleSeconds > 0 && now - latest >= config.idleSeconds * 1000) {
          close('idle');
          return;
        }
        send(': heartbeat\n\n');
      }, config.heartbeatSeconds * 1000);

      // Where the platform reports a disconnect, stop immediately. Lambda
      // never fires this — that is exactly why the idle timeout above is
      // not optional there.
      if (request.signal && typeof request.signal.addEventListener === 'function') {
        request.signal.addEventListener('abort', () => close('client-gone'));
      }
    },

    cancel() {
      open = false;
      if (heartbeat !== null) clearInterval(heartbeat);
      if (typeof stop === 'function') stop();
    },
  });

  return new Response(stream, {
    headers: {
      'content-type': 'text/event-stream',
      'cache-control': 'no-cache, no-transform',
      // Defeats proxy buffering, which turns an SSE stream into a single
      // response delivered at the end.
      'x-accel-buffering': 'no',
    },
  });
}

/** The `?keys=` list, shared by both transports so they cannot disagree. */
function watchedKeys(url) {
  return (url.searchParams.get('keys') || '').split(',').filter((key) => key !== '');
}

/**
 * The polling transport: the same protocol with a zero-length stream.
 *
 * `runtime/store.js`'s `pollTransport` expects a JSON array of the events
 * the stream would have sent. Every key's current value goes out as an
 * `update`, which is idempotent — applying a value a client already holds
 * changes nothing — so no change log is needed to answer correctly. That
 * matters: none of the four stores has one, and the shapes that poll are
 * exactly the ones that cannot hold a stream to compensate.
 */
async function once(url, store) {
  const keys = watchedKeys(url);
  if (keys.length === 0) return json({ error: '`poll` needs `?keys=`' }, 400);

  const seq = Date.now();
  const events = [];
  try {
    for (const key of keys) {
      events.push({ event: 'update', key, value: await store.get(key), seq });
    }
  } catch (error) {
    return json({ error: message(error) }, 500);
  }
  return json(events);
}

/** Change detection for a store with no push channel. */
function poll(store, keys, emit, seconds) {
  const seen = new Map();
  let running = false;
  const tick = async () => {
    if (running) return;
    running = true;
    try {
      for (const key of keys) {
        const value = await store.get(key);
        const encoded = JSON.stringify(value === undefined ? null : value);
        if (seen.has(key) && seen.get(key) === encoded) continue;
        const first = !seen.has(key);
        seen.set(key, encoded);
        if (!first) emit(key, value);
      }
    } catch {
      // A transient store failure must not kill the stream; the next tick
      // retries, and the client is already required to tolerate a close.
    } finally {
      running = false;
    }
  };
  void tick();
  const timer = setInterval(tick, seconds * 1000);
  return () => clearInterval(timer);
}
