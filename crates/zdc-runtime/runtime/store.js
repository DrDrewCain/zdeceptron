// Live sync: the browser half of `durable` placement.
//
// # The transport is a seam, and here is why
//
// Spec §8.1 says holding a stream open is solved on every platform. It is
// not. Checked against vendor documentation:
//
//   Cloudflare Workers   no documented duration limit; billed on CPU, so an
//                        idle stream is nearly free. The best case.
//   Lambda, streaming    900 s hard ceiling, and the full duration is billed
//                        even after the client disconnects.
//   Lambda, buffered     no streaming at all — the response is delivered
//                        only once complete.
//   Lambda behind an ALB no streaming at all — the ALB takes a JSON body,
//                        rejects upgrades, and drops `Transfer-Encoding`.
//   Vercel               300 s Hobby, 800 s Pro.
//   Azure Functions      contested: 230 s total, or a 4-minute *idle*
//                        timeout a heartbeat defeats. Unverified either way.
//   Deno Deploy          no documented timeout, but an isolate can be
//                        evicted mid-stream at any moment.
//
// Two of those shapes cannot hold a stream at all, so a runtime that hard-
// coded `EventSource` would simply not work on them. `subscribe` therefore
// takes a transport, and two are provided: `streamTransport` over
// `EventSource`, and `pollTransport` over `fetch`. They speak the *same*
// protocol — a cursor goes up, ordered events come down — so the difference
// between Cloudflare (never disconnects) and Lambda behind an ALB (cannot
// connect) is a transport choice, not a second code path.
//
// # Resume is load-bearing, not polish
//
// Because the ceiling is 900 s on Lambda, the stream *will* be cut, on a
// timer, in normal operation. Every event carries a sequence number and
// every reconnection sends the last one seen — `Last-Event-ID` for the
// stream transport, `?since=` for the poll transport. That is what makes a
// bounded stream behave like an unbounded one.
//
// When the server cannot prove it has the whole tail a client missed, it
// says `resync` instead of guessing. Continuing silently is the dropped
// update §8.1 forbids, and it is invisible in testing because it only
// happens after a real disconnection.

import { remoteCell } from './rpc.js';
import { decode as decodeValue } from './wire.js';

/** Where the live-sync endpoints live. One place, so the shape is one decision. */
export function liveUrl(keys, cursor) {
  const query = new URLSearchParams();
  query.set('keys', keys.join(','));
  if (cursor !== null && cursor !== undefined) query.set('since', String(cursor));
  return `/_zd/live?${query.toString()}`;
}

export function pollUrl(keys, cursor) {
  const query = new URLSearchParams();
  query.set('keys', keys.join(','));
  if (cursor !== null && cursor !== undefined) query.set('since', String(cursor));
  return `/_zd/poll?${query.toString()}`;
}

// --- the cells ------------------------------------------------------------

/// Every durable key this page reads, and how to update it.
const cells = new Map();

/**
 * A `durable` signal read from the browser.
 *
 * The same `Remote of T` a `server` signal gives, plus a registration: when
 * a write to `key` is announced, the announcement carries the value, so
 * this cell updates without a round trip. That is §17.2.5 fatal 4's
 * `LiveValue` edge, and it is the whole reason two windows move together
 * rather than one of them moving a second later.
 */
export function durable(name, key, inputs) {
  const [read, apply, refetch] = remoteCell(name, inputs);
  let existing = cells.get(key);
  if (!existing) {
    existing = [];
    cells.set(key, existing);
  }
  existing.push({ apply, refetch });
  return read;
}

/** Which keys have a cell. This is what a subscription asks for — never a
 * prefix, because the stores this has to run on do not have prefix watch. */
export function watchedKeys() {
  // `forEach` rather than `Array.from` — see the engine note in `signal.js`.
  const keys = [];
  cells.forEach((_bound, key) => keys.push(key));
  return keys.sort();
}

/** Apply one announced write. Exported so a transport test can drive it. */
export function applyUpdate(key, value) {
  const bound = cells.get(key);
  if (!bound) return false;
  for (const cell of bound) cell.apply(value);
  return true;
}

/**
 * Re-read every cell.
 *
 * The answer to `resync`: the server could not prove it had the whole tail,
 * so nothing about the current values can be trusted and the honest move is
 * to ask again.
 */
export function resyncAll() {
  // `forEach` rather than `for…of` — see the engine note in `signal.js`.
  cells.forEach((bound) => {
    for (const cell of bound) cell.refetch();
  });
}

// --- the protocol ---------------------------------------------------------

/**
 * Route one decoded event.
 *
 * Returns the cursor to resume from. Pure apart from the cell writes, so
 * both transports share it and cannot drift into two dialects.
 */
export function receive(event, cursor) {
  const seq = typeof event.seq === 'number' ? event.seq : undefined;
  // A sequence number that does not advance is an event this client has
  // already seen. Resume is not exact: `Last-Event-ID` and `?since=` both
  // ask for "everything after N", and a server that cannot seek precisely
  // answers from a little earlier — which is allowed, and is why the
  // protocol carries the number at all. Applying such an event replays a
  // value that has since been overwritten, so the page shows the older one
  // until something writes again. It is invisible in testing because it can
  // only happen after a real reconnection.
  const seen = seq !== undefined && typeof cursor === 'number' && seq <= cursor;

  if (event.event === 'resync') {
    // Never skipped. `resync` is the server saying it cannot prove it has
    // the tail this client missed, and re-reading is the answer whether or
    // not the number moved.
    resyncAll();
    return seen ? cursor : (seq ?? cursor);
  }
  if (event.event === 'update') {
    if (seen) return cursor;
    applyUpdate(event.key, event.value);
    return seq ?? cursor;
  }
  // `ready` and anything a newer server invents: advance the cursor if it
  // carried one, and change nothing. An unknown event must not be an error
  // — a browser holding a stale page open across a deploy would then break
  // instead of simply learning nothing new.
  return seen ? cursor : (seq ?? cursor);
}

// --- the two transports ---------------------------------------------------

/**
 * Hold a stream open.
 *
 * `EventSource` reconnects on its own and replays `Last-Event-ID`, which is
 * exactly the resume protocol — so on a platform that can hold a stream,
 * the 900-second cut is handled by the browser and costs one round trip.
 */
export function streamTransport(keys, cursor, onEvent) {
  const source = new EventSource(liveUrl(keys, cursor));
  const handle = (name) => (message) => {
    onEvent(decodeFrame(name, message.data, message.lastEventId));
  };
  for (const name of ['update', 'resync', 'ready']) {
    source.addEventListener(name, handle(name));
  }
  return () => source.close();
}

/**
 * Ask, repeatedly.
 *
 * The fallback for the two shapes that cannot stream at all — Lambda in
 * buffered mode and Lambda behind an ALB — and the cheaper choice anywhere
 * the full stream duration is billed whether or not anyone is listening.
 *
 * It is the same protocol with a zero-length stream: the cursor goes up in
 * the query string instead of a header, and the events come down in an
 * array instead of one at a time.
 */
export function pollTransport(keys, cursor, onEvent, options) {
  const wait = (options && options.interval) || 1000;
  // Resolved with `typeof` rather than named directly: a bare `fetch` in a
  // runtime without one is a `ReferenceError` at subscription time, which
  // would take down module evaluation — and therefore the whole page —
  // over a feature the page can simply do without.
  const fetchImpl =
    (options && options.fetch) || (typeof fetch === 'function' ? fetch : null);
  if (!fetchImpl) {
    // Neither a stream nor a request. The page still works; it just will
    // not learn about another window's writes until something re-reads.
    return () => {};
  }
  const sleep = (options && options.sleep) || ((ms) => new Promise((r) => setTimeout(r, ms)));
  let live = true;
  let at = cursor;

  (async () => {
    while (live) {
      try {
        const response = await fetchImpl(pollUrl(keys, at));
        const events = await response.json();
        for (const event of events) {
          if (!live) return;
          at = onEvent(event);
        }
      } catch (error) {
        // A failed poll is not fatal: the next one carries the same cursor,
        // so nothing is lost by one round trip going missing. Throwing here
        // would end live sync for the life of the page over one dropped
        // packet.
      }
      if (!live) return;
      await sleep(wait);
    }
  })();

  return () => {
    live = false;
  };
}

/** Whether this runtime can hold a stream. */
export function canStream() {
  return typeof EventSource === 'function';
}

/**
 * Start live sync.
 *
 * `transport` defaults to a stream where one is available and a poll where
 * it is not, which is the honest default: the capability differs by
 * platform and by deployment shape, and a page cannot know which it is
 * behind.
 */
export function subscribe(options) {
  const settings = options || {};
  const keys = settings.keys || watchedKeys();
  if (keys.length === 0) return () => {};

  let cursor = settings.since === undefined ? null : settings.since;
  const transport = settings.transport || (canStream() ? streamTransport : pollTransport);
  const onEvent = (event) => {
    cursor = receive(event, cursor);
    return cursor;
  };
  return transport(keys, cursor, onEvent, settings);
}

/**
 * One `event:`/`data:` pair, as an object both transports produce.
 *
 * Named `decodeFrame` rather than `decode` because `wire.js` exports a
 * `decode` too, and the two do different jobs at different layers: this
 * one turns an SSE frame into an event, that one turns JSON into a ZD
 * value. Two `decode`s one import apart is a name collision waiting for
 * whichever file gets flattened into the other's scope.
 */
export function decodeFrame(name, data, lastEventId) {
  let payload = {};
  try {
    payload = JSON.parse(data);
  } catch (error) {
    payload = {};
  }
  const seq =
    typeof payload.seq === 'number'
      ? payload.seq
      : lastEventId === undefined || lastEventId === null || lastEventId === ''
        ? undefined
        : Number(lastEventId);
  // Decoded here rather than at the cell, because this is the one place
  // bytes become values — and a `Map` pushed to a second window has to
  // arrive as a `Map`, not as the `{"$map":[...]}` it travelled as.
  //
  // A frame this runtime cannot decode becomes a `resync` rather than an
  // exception. The alternative is a throw out of an `EventSource` listener,
  // which nothing catches; and the alternative to *that* is applying a
  // value we could not read, which is the dropped update §8.1 forbids.
  // Asking again is the only answer that is neither.
  let value = null;
  try {
    if (payload.value !== undefined) value = decodeValue(payload.value);
  } catch (error) {
    return { event: 'resync', seq: Number.isFinite(seq) ? seq : undefined, key: undefined, value: null };
  }
  return {
    event: name,
    seq: Number.isFinite(seq) ? seq : undefined,
    key: payload.key,
    value,
  };
}
