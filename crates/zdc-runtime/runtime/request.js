// The outbound request a `request` declaration is — issue #19.
//
// Its own module, and not part of `rpc.js`, because the two are different
// promises. `rpc.js` talks to an endpoint this compiler emitted, on this
// origin, whose body it wrote and can therefore read as its own; this file
// talks to a host the *program* named, whose answer nobody vouches for. A
// program that declares no `request` must not ship the one `fetch` in the
// runtime that can name a host, and keeping them in separate files is what
// makes that a fact about the bytes rather than about a code path (§16.3.1).
//
// Four properties are this file's alone to keep, and each is the reason a
// clause of the language's design was not needed:
//
//  1. **The method is `GET` and there is no body.** A `Remote of T` is a
//     read. A request that changed something on a third party would be a
//     command, and commands are handler statements with an outcome cell,
//     not signal initialisers.
//  2. **The headers are `HEADERS` below and nothing else.** No program
//     value reaches one, which is why the language has no header clause
//     for a credential to be written into.
//  3. **The query string is built here, with `encodeURIComponent`.** A
//     value cannot leave its own parameter, so it cannot add a parameter,
//     change the path, or reach the host.
//  4. **A `Failed` message is composed from this file's own control flow.**
//     `rpc.js` reads `body.error` out of a failed response, which is right
//     for a body it wrote and wrong for one it did not: a third party
//     would otherwise choose text a program renders.

import { signal, effect } from './signal.js';

const LOADING = { tag: 'Loading', fields: [] };

function ready(value) {
  return { tag: 'Ready', fields: [value] };
}

/**
 * The three codes, the same closed set `rpc.js` uses and `zdc-types`'s
 * `FailureCode` pins. Every one of them is decided by *this file's*
 * control flow, so none is a channel the answering host can write into:
 * `Unreachable` means no response object came back, `Timeout` means the
 * deadline below fired, and `Rejected` means a response came back and its
 * status line said no.
 */
const CODES = Object.freeze({
  UNREACHABLE: 'Unreachable',
  TIMEOUT: 'Timeout',
  REJECTED: 'Rejected',
});

/**
 * Every header the request carries, frozen.
 *
 * `accept` and nothing else. There is deliberately no way to add one: an
 * `Authorization` header is the shortest path from a credential to a third
 * party, and the compiler's answer to that route is that the route does
 * not exist. A cross-origin request with a header outside CORS's
 * safelisted set also needs a preflight the other host has to answer, so a
 * header clause would mostly be a way to fail.
 */
const HEADERS = Object.freeze({ accept: 'text/plain, application/json' });

/** How long a request may take before the runtime stops waiting. */
const DEADLINE_MS = 30000;

/**
 * The URL a request is sent to: the destination, then the parameters.
 *
 * `encodeURIComponent` on both halves of every pair, so a value is a
 * value. Without it `with q is "a&admin=1"` would be two parameters, and
 * a value holding `#` would truncate the query — neither is a leak on its
 * own, and both are the shape of one.
 *
 * The destination is **not** encoded and must not be: it arrived as a
 * literal that `zdc_hir::destination` already parsed into a scheme, a host
 * and a path, and encoding it would turn its slashes into `%2F`.
 */
export function requestUrl(destination, pairs) {
  if (pairs.length === 0) return destination;
  const query = pairs
    .map(([name, value]) => `${encodeURIComponent(name)}=${encodeURIComponent(value)}`)
    .join('&');
  return `${destination}?${query}`;
}

/**
 * A failure this file classified. Nothing else constructs one, and the
 * code travels on the object rather than in the message, so nothing
 * downstream recovers it by parsing text.
 */
class RequestFailure extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'RequestFailure';
    this.zdCode = code;
  }
}

function codeOf(error) {
  if (error instanceof RequestFailure) return error.zdCode;
  const name = error && error.name;
  if (name === 'AbortError' || name === 'TimeoutError') return CODES.TIMEOUT;
  return CODES.UNREACHABLE;
}

/**
 * The `Failed` payload.
 *
 * **The message is this file's sentence, not the host's.** A
 * `RequestFailure` carries text composed below from the destination and
 * the status line; anything else — a transport a host page replaced, a
 * `TypeError` from the platform — is reported as its `name` and no more.
 * `String(error.message)` is what `rpc.js` writes, and it is what would
 * let an answering host put its own prose on the page.
 */
function failed(destination, error) {
  const message =
    error instanceof RequestFailure
      ? error.message
      : `${destination} could not be reached (${(error && error.name) || 'error'})`;
  return {
    tag: 'Failed',
    fields: [{ message, code: { tag: codeOf(error), fields: [] } }],
  };
}

/** A cancellable deadline, or a no-op where the platform has no timers. */
function startDeadline() {
  const has = typeof AbortController === 'function' && typeof setTimeout === 'function';
  if (!has) return { signal: undefined, cancel: () => {}, expired: () => false };
  const controller = new AbortController();
  let fired = false;
  const timer = setTimeout(() => {
    fired = true;
    controller.abort();
  }, DEADLINE_MS);
  return {
    signal: controller.signal,
    cancel: () => clearTimeout(timer),
    // This file's own boolean, so nothing on the wire chooses the code.
    expired: () => fired,
  };
}

let transport = defaultTransport;

/** Replace the transport. Used by tests, which answer without a network. */
export function setRequestTransport(next) {
  transport = next || defaultTransport;
}

async function defaultTransport(url) {
  const deadline = startDeadline();
  let response;
  try {
    response = await fetch(url, {
      // Written out rather than defaulted, so a reader of this file can
      // see there is no method and no body to be changed.
      method: 'GET',
      headers: HEADERS,
      body: undefined,
      // `omit`, so a browser sends neither cookies nor HTTP credentials to
      // the host the program named. A same-origin destination gets the
      // same treatment: an endpoint is `rpc.js`'s business, and this file
      // has no reason to carry anybody's session.
      credentials: 'omit',
      signal: deadline.signal,
    });
  } catch (error) {
    throw deadline.expired()
      ? new RequestFailure(CODES.TIMEOUT, `${url} did not answer within ${DEADLINE_MS}ms`)
      : new RequestFailure(CODES.UNREACHABLE, `${url} could not be reached`);
  } finally {
    deadline.cancel();
  }
  if (!response.ok) {
    // The status line, and not the body. A number the host chose out of a
    // set the protocol fixed is a far smaller channel than prose it wrote.
    throw new RequestFailure(CODES.REJECTED, `${url} answered with ${response.status}`);
  }
  // `text()`, never `json()`: the language says a request gives `Text`, so
  // there is nothing to decode and no decoder to disagree with a host.
  return response.text();
}

/**
 * A `request` declaration, as the getter of `Remote of Text` it is.
 *
 * `pairs` is `[[name, getter], …]` in source order. The getters are read
 * **inside** the effect, which is what makes the request re-run when — and
 * only when — one of its arguments changes.
 *
 * Generation-guarded for the reason `remoteCell` is: typing into a bound
 * signal starts a request per keystroke, and the first answer must not
 * overwrite the last.
 */
export function request(destination, pairs) {
  const [read, write] = signal(LOADING);
  let generation = 0;

  effect(() => {
    const resolved = pairs.map(([name, get]) => [name, String(get())]);
    const url = requestUrl(destination, resolved);
    const mine = ++generation;
    write(LOADING);
    Promise.resolve()
      .then(() => transport(url))
      .then(
        (text) => {
          if (mine === generation) write(ready(text));
        },
        (error) => {
          if (mine === generation) write(failed(destination, error));
        },
      );
  });

  return read;
}
