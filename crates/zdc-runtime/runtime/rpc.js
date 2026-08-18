// The client half of the boundary the compiler derived.
//
// Nothing here decides anything. Which endpoints exist, what they are
// called and what they take is settled by the tier split (spec §17.2);
// this file only moves the bytes and keeps the `Remote of T` variant
// honest while they are in flight.

import { signal, effect } from './signal.js';
import { stringify, decode } from './wire.js';

const LOADING = { tag: 'Loading', fields: [] };

function ready(value) {
  return { tag: 'Ready', fields: [value] };
}

/**
 * The closed set of `Failed` codes, mirrored in `zdc-types`'s
 * `FailureCode` (crates/zdc-types/src/failure.rs) and pinned against it by
 * a test. Three, not four: `Malformed` was specified and dropped, because
 * a code a server selects by choosing what it writes into a body is a bit
 * of channel at a public label.
 *
 * These are the arms of the built-in `choice` called `Code`, so each
 * string below is a variant *tag* rather than a value a program compares
 * text against. The pinning test reads them off that choice, so it fails
 * if this object and the language's arms ever name different sets.
 *
 * `code` is public by construction, and this is the construction: every
 * one of these is decided by *this file's* control flow. `Unreachable`
 * means no response object came back at all; `Timeout` means the deadline
 * below fired, which is our own `setTimeout` and our own boolean;
 * `Rejected` means a response object came back and the status line or the
 * decoder rejected it. None of them is read out of the response body, and
 * none of them is read out of an error's text.
 */
const CODES = Object.freeze({
  UNREACHABLE: 'Unreachable',
  TIMEOUT: 'Timeout',
  REJECTED: 'Rejected',
});

/**
 * A failure the transport classified. Nothing else constructs one.
 *
 * The code travels on the *object*, chosen at the throw site, so nothing
 * downstream has to parse a message to recover it — which is the only way
 * a server's bytes could have reached the field.
 */
class TransportFailure extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'TransportFailure';
    this.zdCode = code;
  }
}

/**
 * Which code a rejection carries.
 *
 * A transport this runtime did not write — a test's, or a host page's —
 * rejects with whatever it likes. An abort is a `Timeout` because that is
 * what an abort of an RPC is; anything else is `Unreachable`, because no
 * answer was obtained and the runtime has no evidence of one. The message
 * is never consulted.
 */
function codeOf(error) {
  if (error instanceof TransportFailure) return error.zdCode;
  const name = error && error.name;
  if (name === 'AbortError' || name === 'TimeoutError') return CODES.TIMEOUT;
  return CODES.UNREACHABLE;
}

/**
 * The `Failed` payload: two fields at two labels.
 *
 * `message` is host text and carries §14G.1.3(d)'s join — as secret as
 * whatever the endpoint read. `code` is the runtime's own verdict on the
 * transport and is `public`. The compiler enforces the difference; this
 * file's job is to make the second one true.
 *
 * `code` is a value of `Code`, a built-in `choice`, so it travels in the
 * same shape every other variant does — `{ tag, fields }`, as `variant()`
 * in `dom.js` builds and as `whenInto` dispatches on. It was a bare
 * string until `Code` became a type, and a bare string is what let
 * `error.code is "Timout"` compile.
 */
function failed(error) {
  return {
    tag: 'Failed',
    fields: [
      {
        message: String(error && error.message ? error.message : error),
        code: { tag: codeOf(error), fields: [] },
      },
    ],
  };
}

/**
 * A `server` or `durable` signal read from the browser.
 *
 * Returns a getter of `Remote of T`, which is exactly what §5.2 says the
 * read yields: the network is in the value because the network is there,
 * and the caller cannot reach the value without eliminating the variant.
 *
 * `inputs` are the getters for the endpoint's parameters, in the wire
 * order the manifest records. Reading them inside the effect is what
 * makes the call re-run when — and only when — one of them changes.
 */
export function remote(name, inputs) {
  return remoteCell(name, inputs)[0];
}

/**
 * The same cell, with the handles live sync needs.
 *
 * Returns `[read, apply, refetch, fail]`:
 *
 * - `read` is the getter `remote` returns.
 * - `apply(value)` writes a value straight in, for an update the server
 *   *pushed*. Without it a second window would have to re-fetch on every
 *   announcement, which is the round trip §17.2.5 fatal 4's `LiveValue`
 *   edge exists to avoid.
 * - `refetch()` re-runs the call, for a `resync` — the case where the
 *   server cannot prove it has the whole tail a client missed and the
 *   only honest answer is to ask again.
 * - `fail(error)` puts the cell in `Failed`, for when live sync has
 *   stopped trying. A `durable` read claims the cell tracks the store;
 *   once nothing is watching, a `Ready` holding the last value seen is
 *   that claim still being made and no longer true. See `store.js`'s
 *   retry policy for when it fires.
 *
 * All three bump the generation counter, so a push — or a give-up — that
 * lands while a request is in flight is not overwritten by that request's
 * late answer.
 */
export function remoteCell(name, inputs) {
  const [read, write] = signal(LOADING);
  // Generation-guarded: typing `ab` and having the first response land
  // last must not overwrite the newer result.
  let generation = 0;
  let latest = () => {};

  effect(() => {
    const args = inputs.map((input) => input());
    latest = () => start(args);
    start(args);
  });

  function start(args) {
    const mine = ++generation;
    write(LOADING);
    invoke(name, args).then(
      (value) => {
        if (mine === generation) write(ready(value));
      },
      (error) => {
        if (mine === generation) write(failed(error));
      },
    );
  }

  function apply(value) {
    // Claims the generation, so an older request landing later is ignored:
    // a pushed value is newer than anything already on the wire.
    generation += 1;
    write(ready(value));
  }

  function refetch() {
    latest();
  }

  function fail(error) {
    // Claims the generation for the reason `apply` does, and a sharper one:
    // a request still on the wire when the connection was declared lost
    // must not land afterwards and put the cell back in `Ready`, making a
    // page that has stopped receiving updates look as though it had not.
    generation += 1;
    write(failed(error));
  }

  return [read, apply, refetch, fail];
}

/**
 * A cross-region write: the browser asks the server to perform it.
 *
 * The right-hand side and every index were evaluated in the browser and
 * are shipped as arguments; only the place resolution and the store
 * operator run on the other side (spec §17.2.7's command rule).
 *
 * Returns a promise, and generated handlers `await` it. That is not a
 * convenience: a discarded promise means the handler cannot order two
 * writes, cannot see either fail, and half-applies in silence.
 *
 * **Generated handlers no longer call this.** One write per request is one
 * store operation per request, and a handler with three of them can
 * half-apply however carefully each one is awaited. `atomic` replaced it.
 * The export stays because it is the one-write shape of the same request
 * and a host page or a test may want it.
 */
export function call(name, ...args) {
  return invoke(name, args);
}

/** The reserved name the batch is posted to. `~` cannot appear in a ZD
 * identifier, so it can never collide with an endpoint. */
export const ATOMIC = '~atomic';

/**
 * Every durable write one handler asked for, as one transaction.
 *
 * `commands` is `[[endpoint, args], ...]` in source order, which is the
 * list the generated handler accumulated in `$tx`. The server runs all of
 * them and commits them in a single store transaction, so they all land or
 * none does — and a failure part way through leaves the store as it was
 * rather than half-written.
 *
 * The list can be built in the browser at all because §17.2.7's Command
 * rule evaluated every right-hand side and index here, so by the time this
 * is called the whole transaction is decided and nothing in it depends on
 * reading the server's state. That is what lets the server use a
 * non-interactive atomic batch, which is the only kind Deno KV and
 * DynamoDB have.
 *
 * An empty list is not a request. A handler whose only write sits inside
 * an `if` that did not fire has nothing to commit, and a round trip to say
 * so would be a request per click on every conditional write in a program.
 */
export function atomic(commands) {
  if (!commands || commands.length === 0) return Promise.resolve(null);
  return invoke(ATOMIC, commands);
}

/**
 * Where a write's failure goes.
 *
 * A generated handler wraps its awaited writes in `try`/`catch` and calls
 * this. It exists because the alternative is an unhandled rejection: the
 * DOM layer invokes a listener and discards what it returns, so an async
 * handler that rejects produces — at best — a console entry nobody reads,
 * and at worst nothing at all.
 *
 * This is deliberately not "show the user an error". The language has no
 * global error surface, and inventing one here would be a UI decision made
 * in the runtime. What it guarantees is that the failure is *reachable*:
 * the default reports it through the platform's own channel, and an
 * application — or a test — can replace the sink.
 */
let failureSink = defaultFailureSink;

// Named `reportFailure` and not `failed`: `failed` is already the private
// constructor for the `Failed` variant three lines from the top of this
// file, and a second declaration of that name silently replaces it — so
// `write(failed(error))` would store `undefined` and the page would sit in
// `Loading` for ever. That is exactly the bug this whole sink exists to
// prevent, arriving through the fix for it.
export function reportFailure(error) {
  failureSink(error);
}

/** Replace the failure sink. Used by tests, and by a host page that has
 * somewhere better to put it than the console. */
export function setFailureSink(next) {
  failureSink = next || defaultFailureSink;
}

function defaultFailureSink(error) {
  // `reportError` is the platform's own "this went wrong and nobody caught
  // it" channel — it reaches `window.onerror` and error-reporting services
  // the way a genuinely uncaught exception would. `console.error` is the
  // fallback for runtimes that predate it.
  if (typeof reportError === 'function') {
    reportError(error);
  } else if (typeof console !== 'undefined' && console.error) {
    console.error(error);
  }
}

/** Which endpoint URL a name maps to. One place, so the shape is one decision. */
export function endpointUrl(name) {
  return `/_zd/${encodeURIComponent(name)}`;
}

let transport = defaultTransport;

/** Replace the transport. Used by tests, which record calls rather than make them. */
export function setTransport(next) {
  transport = next || defaultTransport;
}

function invoke(name, args) {
  try {
    return Promise.resolve(transport(name, args));
  } catch (error) {
    return Promise.reject(error);
  }
}

/**
 * How long a call may take before the runtime stops waiting.
 *
 * A deadline is what makes `Timeout` a thing this file decides rather
 * than a thing it reports. Without one, a stalled server is
 * indistinguishable from a slow one for ever, and the arm never fires.
 */
const DEADLINE_MS = 30000;

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
    // Read from our own variable and not from the abort reason, so that
    // nothing on the wire participates in the answer.
    expired: () => fired,
  };
}

async function defaultTransport(name, args) {
  const deadline = startDeadline();
  let response;
  try {
    response = await fetch(endpointUrl(name), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      // `stringify`, never `JSON.stringify`: a `Map of K to V` is a
      // JavaScript `Map`, and `JSON.stringify` turns one into `{}` without
      // saying so. See `wire.js`.
      body: stringify(args),
      signal: deadline.signal,
    });
  } catch (error) {
    // No response object: nothing was received, so nothing the server
    // could have sent chose this. Which of the two codes it is comes from
    // `deadline.expired()`, this file's own boolean.
    throw deadline.expired()
      ? new TransportFailure(CODES.TIMEOUT, `${name} did not answer within ${DEADLINE_MS}ms`)
      : new TransportFailure(CODES.UNREACHABLE, `${name} could not be reached: ${error}`);
  } finally {
    deadline.cancel();
  }
  if (!response.ok) {
    // The body carries why. A `Remote of T` renders that text, so losing
    // it here would turn "`GREETING_API_KEY` is not set" into "500".
    // It goes into `message`, which is labelled; the *code* comes from
    // the status line, which is not part of the body.
    throw new TransportFailure(CODES.REJECTED, await reason(response, name));
  }
  try {
    return decode(await response.json());
  } catch (error) {
    // A 2xx the decoder could not read. `Rejected` again, deliberately:
    // it is the same code a non-2xx status line produces, so choosing
    // what to write into a 200 body distinguishes nothing that the status
    // line cannot already distinguish on its own. That equality is what
    // keeps the body out of `code`.
    throw new TransportFailure(CODES.REJECTED, `${name} answered with something unreadable: ${error}`);
  }
}

async function reason(response, name) {
  try {
    const body = await response.json();
    if (body && typeof body.error === 'string') return body.error;
  } catch (error) {
    // Not JSON. Fall through to the status, which is all there is.
  }
  return `${name} failed with ${response.status}`;
}
