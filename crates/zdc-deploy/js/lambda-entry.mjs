// AWS Lambda entry, over a Function URL in `RESPONSE_STREAM` invoke mode.
//
// This is the target that disproves "one artifact, no per-platform build",
// and it is worth being precise about why. Two things here are not web
// APIs and have no ECMA-429 equivalent:
//
//   1. `awslambda.streamifyResponse` — a global the Node runtime injects,
//      with no import and no standard counterpart. Buffered mode cannot
//      stream at all, so without it live sync is impossible, not merely
//      slow.
//   2. `responseStream` is a **Node.js writable stream**, not a WHATWG
//      `WritableStream`. There is no `pipeTo`; the loop below is the type
//      mismatch made visible.
//
// The other thing to know is commercial rather than technical. Lambda bills
// the full duration of a streamed response and does **not** stop when the
// client disconnects, and `request.signal` never fires here. The idle
// timeout in `_zd/config.js` is the only thing standing between a closed
// browser tab and a bill for the rest of the function timeout.

import { endpoints } from './_zd/endpoints.js';
import { config } from './_zd/config.js';
import { json, route } from './_zd/router.js';
import { store } from './_zd/store.js';

const env = (key) => process.env[key];

/** A Function URL payload (format 2.0) as a web `Request`. */
function toRequest(event) {
  const query = event.rawQueryString ? `?${event.rawQueryString}` : '';
  const host = event.headers?.host ?? 'lambda.invalid';
  const url = `https://${host}${event.rawPath ?? '/'}${query}`;
  const method = event.requestContext?.http?.method ?? 'GET';
  const body =
    method === 'GET' || method === 'HEAD' || event.body === undefined || event.body === null
      ? undefined
      : event.isBase64Encoded
        ? Uint8Array.from(atob(event.body), (character) => character.charCodeAt(0))
        : event.body;
  return new Request(url, { method, headers: event.headers ?? {}, body });
}

export const handler = awslambda.streamifyResponse(async (event, responseStream) => {
  const request = toRequest(event);
  const response =
    (await route(request, endpoints, store, env, config)) ?? json({ error: 'not found' }, 404);

  const out = awslambda.HttpResponseStream.from(responseStream, {
    statusCode: response.status,
    headers: Object.fromEntries(response.headers),
  });

  if (response.body === null) {
    out.end();
    return;
  }
  const reader = response.body.getReader();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      // Node back-pressure, because `out` is a Node writable stream.
      if (!out.write(value)) await new Promise((resolve) => out.once('drain', resolve));
    }
  } finally {
    out.end();
  }
});
