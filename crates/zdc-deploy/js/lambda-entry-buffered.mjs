// AWS Lambda entry, behind an Application Load Balancer.
//
// An ALB does not stream to a Lambda target at all: it invokes the function
// and expects one JSON response object of at most 1 MB, it does not honour
// hop-by-hop headers such as `Transfer-Encoding`, and it rejects upgrade
// requests with HTTP 400. So nothing here calls into Lambda's response
// streaming path, and nothing here can.
//
// A program with durable state is refused at build time for this front,
// because live sync is a held-open `text/event-stream` and none of that is
// available here. This file exists for the case that remains: a program
// whose server work is request/response only.

import { endpoints } from './_zd/endpoints.js';
import { config } from './_zd/config.js';
import { json, route } from './_zd/router.js';
import { store } from './_zd/store.js';

const env = (key) => process.env[key];

/** An ALB target-group payload as a web `Request`. */
function toRequest(event) {
  const parameters = new URLSearchParams(event.queryStringParameters ?? {}).toString();
  const host = event.headers?.host ?? 'lambda.invalid';
  const url = `https://${host}${event.path ?? '/'}${parameters ? `?${parameters}` : ''}`;
  const method = event.httpMethod ?? 'GET';
  const body =
    method === 'GET' || method === 'HEAD' || event.body === undefined || event.body === null
      ? undefined
      : event.isBase64Encoded
        ? Uint8Array.from(atob(event.body), (character) => character.charCodeAt(0))
        : event.body;
  return new Request(url, { method, headers: event.headers ?? {}, body });
}

export const handler = async (event) => {
  const request = toRequest(event);
  const response =
    (await route(request, endpoints, store, env, config)) ?? json({ error: 'not found' }, 404);
  return {
    statusCode: response.status,
    statusDescription: `${response.status}`,
    headers: Object.fromEntries(response.headers),
    isBase64Encoded: false,
    body: await response.text(),
  };
};
