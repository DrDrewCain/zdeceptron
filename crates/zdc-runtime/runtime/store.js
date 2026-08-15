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
//
// # And the reconnection is bounded (#143)
//
// Resume says *how* to come back; it does not say how often, or for how
// long. Unstated, both answers were "for ever, as fast as before": the
// poll loop asked again every second whatever happened to the last request,
// and the stream handed reconnection to `EventSource`, which retries at a
// fixed interval with no jitter and no end. An outage disconnects every
// open tab at once, so that is every open tab returning together, every few
// seconds, for as long as the outage lasts.
//
// The policy under "the retry policy" below is the bound: exponential
// backoff from 1 s, a 30 s ceiling, full jitter, and a give-up after eight
// consecutive failures. What the program sees when it gives up is a
// `Failed` in every durable cell — `failAll` — because a `Ready` nothing is
// keeping up to date is the language making a claim it has stopped
// honouring.

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
  const [read, apply, refetch, fail] = remoteCell(name, inputs);
  let existing = cells.get(key);
  if (!existing) {
    existing = [];
    cells.set(key, existing);
  }
  existing.push({ apply, refetch, fail });
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

/**
 * Tell every cell that live sync has stopped.
 *
 * **This is what a program observes when the policy below gives up**, and
 * the reason it is a write rather than a log line. `Remote of T` has three
 * arms and a `when` renders one of them; while sync is merely *retrying*,
 * the last value is still the right one to show, because the cursor means
 * a reconnection will be handed the tail it missed. Once the retries are
 * spent that guarantee is gone and the value on screen is of unknown age
 * with nothing coming to correct it. Leaving it in `Ready` would be the
 * silent stall — a page that looks live and is not — so the cell moves to
 * `Failed` and the program's third arm gets to say so.
 *
 * `error` is a plain `Error`, which `rpc.js`'s `codeOf` classifies as
 * `Unreachable`. That is the honest code: no answer was obtained, and none
 * of the three codes is being read out of anything a server sent.
 */
export function failAll(error) {
  cells.forEach((bound) => {
    for (const cell of bound) cell.fail(error);
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

// --- the retry policy -----------------------------------------------------
//
// A client that reconnects without a bound is a load generator, and the
// moment it generates load is the moment a server can least take it: an
// outage disconnects every open tab at once, so every open tab comes back
// at once. The numbers below are the bound, and each is here for a reason.

/** The first delay, and the poll transport's steady-state interval.
 *
 * One second, because that is what the poll interval already was: a single
 * dropped request therefore costs exactly what it costs today, and the
 * policy only starts to differ once failures repeat. */
const RETRY_BASE_MS = 1000;

/** The longest wait between attempts, whatever the doubling reaches.
 *
 * Thirty seconds, for two reasons that agree. It is the sustained floor a
 * recovering server has to survive — ten thousand open tabs at a 30 s
 * ceiling are ~333 requests a second, which a deployment can be sized for,
 * where the same tabs at the 1 s interval are 10,000 a second and the
 * recovery does not happen. And it is `DEADLINE_MS` in `rpc.js`: the
 * longest a client waits before asking again is the longest it is willing
 * to wait for one answer, so the two bounds do not disagree by an order of
 * magnitude. */
const RETRY_CEILING_MS = 30000;

/** How many consecutive failures before sync gives up.
 *
 * Eight. With the schedule above and full jitter that is between zero and
 * 121 seconds of trying, ~60 s expected — long enough to ride out a
 * restart, a deploy or a failover, all of which are seconds; short enough
 * that a real outage does not leave every tab that was open when it began
 * asking for the length of it. Outages last hours. A give-up is not the
 * page breaking: `failAll` puts the cells in `Failed`, which is an arm the
 * program already had to write. */
const RETRY_LIMIT = 8;

/**
 * How long to wait before attempt `attempt` (counted from zero).
 *
 * Exponential, capped, and **jittered** — the delay is drawn uniformly from
 * `[0, bound)` rather than being the bound. The jitter is not decoration.
 * Every client that dropped at the same moment is a client whose backoff
 * schedule starts at the same moment, so an unjittered schedule has all of
 * them return together, fail together, and return together again at each
 * doubling: the herd survives the backoff intact and knocks the server over
 * a second time. Drawing from the whole interval spreads the arrivals and
 * halves the expected rate, and is what the AWS Architecture Blog's
 * *Exponential Backoff And Jitter* calls "full jitter".
 *
 * `random` is a seam so a test can be deterministic: `Math.random` is the
 * default and the only thing a browser uses.
 */
export function backoffMs(attempt, random) {
  const roll = random || Math.random;
  const bound = Math.min(RETRY_CEILING_MS, RETRY_BASE_MS * Math.pow(2, attempt));
  return Math.floor(roll() * bound);
}

/**
 * The bound, as one object both transports hold.
 *
 * Shared rather than written twice because a stream that gives up after
 * eight tries and a poll that gives up after twelve would be two policies
 * wearing one name, and the difference would only ever show up during an
 * outage — which is the one time nobody is reading the code.
 *
 * `ok()` is called when something arrives, and *that* is the definition of
 * success: a connection that opens and delivers nothing is not a working
 * connection, and a stream cut at its duration ceiling — which on Lambda
 * happens every 900 s in normal operation — delivers the current value of
 * every watched key as soon as it reopens, so the ordinary case resets the
 * count and never approaches the limit.
 */
function attempts(options) {
  const random = options && options.random;
  const sleep =
    (options && options.sleep) || ((ms) => new Promise((r) => setTimeout(r, ms)));
  let failures = 0;
  return {
    ok: () => {
      failures = 0;
    },
    // Resolves `true` when it is worth trying again and `false` when the
    // policy is spent — and in that case the cells have already been told,
    // so a caller only has to stop.
    next: () => {
      failures += 1;
      if (failures >= RETRY_LIMIT) {
        failAll(new Error(`live sync gave up after ${failures} attempts`));
        return Promise.resolve(false);
      }
      return sleep(backoffMs(failures - 1, random)).then(() => true);
    },
  };
}

// --- the two transports ---------------------------------------------------

/**
 * Hold a stream open.
 *
 * **The reconnection is this file's and not `EventSource`'s**, which is a
 * change from letting the browser do it. `EventSource` reconnects by
 * itself, after a fixed delay, with no jitter, for ever — the unbounded
 * retry above exists to replace exactly that — and it reconnects to the URL
 * it was constructed with, so its `?since=` is the cursor this client
 * started from rather than the one it reached. The server then replays
 * everything after that point on every attempt, which `receive` correctly
 * discards as already seen and which nobody is paid to send.
 *
 * So the source is closed on the first error and reopened here, at `at`,
 * under the policy. The 900-second cut still costs one round trip; what it
 * no longer costs is a replay from the beginning of the session.
 */
export function streamTransport(keys, cursor, onEvent, options) {
  const retry = attempts(options);
  let live = true;
  let at = cursor;
  let source = null;

  const open = () => {
    // Held in a local as well as in `source`, because the listeners below
    // outlive the connection they were installed on. Reading the outer
    // variable from inside them would mean a late error from a source
    // already replaced closes its *successor* and books a second
    // reconnection — one dropped stream retried twice, for ever, which is
    // the failure this whole policy is here to prevent.
    const active = new EventSource(liveUrl(keys, at));
    source = active;
    const handle = (name) => (message) => {
      // A frame is the proof that the connection works, and the only
      // proof there is: this is where the failure count goes back to zero.
      retry.ok();
      at = onEvent(decodeFrame(name, message.data, message.lastEventId));
    };
    for (const name of ['update', 'resync', 'ready']) {
      active.addEventListener(name, handle(name));
    }
    active.addEventListener('error', () => {
      active.close();
      if (!live || source !== active) return;
      retry.next().then((again) => {
        if (live && again) open();
      });
    });
  };
  open();

  return () => {
    live = false;
    if (source) source.close();
  };
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
  const wait = (options && options.interval) || RETRY_BASE_MS;
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
  const retry = attempts(options);
  let live = true;
  let at = cursor;

  (async () => {
    while (live) {
      let answered = false;
      try {
        const response = await fetchImpl(pollUrl(keys, at));
        // A status line is an answer and a 5xx is not a good one. Without
        // this an outage that refuses in two milliseconds is polled as
        // fast as the loop can run — the failure is only a failure if
        // something calls it one.
        if (!response.ok) throw new Error(`poll answered ${response.status}`);
        const events = await response.json();
        for (const event of events) {
          if (!live) return;
          at = onEvent(event);
        }
        answered = true;
      } catch (error) {
        // A failed poll is not fatal: the next one carries the same cursor,
        // so nothing is lost by one round trip going missing. Throwing here
        // would end live sync for the life of the page over one dropped
        // packet. What is not free is *how soon* the next one goes, and
        // how many of them there are — which is what `retry` decides.
      }
      if (!live) return;
      if (answered) {
        retry.ok();
        await sleep(wait);
      } else if (!(await retry.next())) {
        return;
      }
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
 *
 * The whole `settings` object reaches the transport, so `sleep` and
 * `random` — the two seams the retry policy is written against — are
 * settable from here. Nothing else needs them: a browser has a clock and a
 * random number generator, and a test has neither that it is willing to
 * wait for.
 *
 * The returned function stops the subscription. It is not the only thing
 * that ends one: after `RETRY_LIMIT` consecutive failures the transport
 * stops on its own and every durable cell moves to `Failed`. That is
 * terminal for the life of the page, which is the point — a program that
 * wants to keep asking has an arm to render that in.
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
