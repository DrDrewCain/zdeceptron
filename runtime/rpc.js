// The client half of the boundary the compiler derived.
//
// Nothing here decides anything. Which endpoints exist, what they are
// called and what they take is settled by the tier split (spec §17.2);
// this file only moves the bytes and keeps the `Remote of T` variant
// honest while they are in flight.

import { signal, effect } from './signal.js';

const LOADING = { tag: 'Loading', fields: [] };

function ready(value) {
  return { tag: 'Ready', fields: [value] };
}

function failed(error) {
  return { tag: 'Failed', fields: [{ message: String(error && error.message ? error.message : error) }] };
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
  const [read, write] = signal(LOADING);
  // Generation-guarded: typing `ab` and having the first response land
  // last must not overwrite the newer result.
  let generation = 0;

  effect(() => {
    const args = inputs.map((input) => input());
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
  });

  return read;
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
 */
export function call(name, ...args) {
  return invoke(name, args);
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

async function defaultTransport(name, args) {
  const response = await fetch(endpointUrl(name), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(args),
  });
  if (!response.ok) {
    throw new Error(`${name} failed with ${response.status}`);
  }
  return response.json();
}
