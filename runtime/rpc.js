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
 */
export function call(name, ...args) {
  return invoke(name, args);
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
