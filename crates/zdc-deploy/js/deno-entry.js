// Deno Deploy entry.
//
// The handler shape is web-standard; `Deno.serve`, `Deno.env` and the
// static file read are not, and there is no ECMA-429 spelling for any of
// them — the CLI API proposal that would cover environment variables is
// still a proposal.
//
// The platform constraint to design against is not a timeout. There is no
// documented request timeout, and streaming response bytes is itself what
// keeps the app alive; but an isolate can be evicted at any time, including
// mid-stream, with a SIGINT and five seconds. Client reconnect is mandatory
// here rather than merely advisable.

import { endpoints } from './_zd/endpoints.js';
import { config } from './_zd/config.js';
import { route } from './_zd/router.js';
import { store } from './_zd/store.js';

const TYPES = {
  html: 'text/html; charset=utf-8',
  js: 'text/javascript; charset=utf-8',
  css: 'text/css; charset=utf-8',
  json: 'application/json',
  svg: 'image/svg+xml',
};

async function asset(url) {
  const path = url.pathname === '/' ? '/index.html' : url.pathname;
  if (path.includes('..')) return new Response('not found', { status: 404 });
  try {
    const file = await Deno.readFile(`./public${path}`);
    const extension = path.slice(path.lastIndexOf('.') + 1);
    return new Response(file, {
      headers: { 'content-type': TYPES[extension] ?? 'application/octet-stream' },
    });
  } catch {
    return new Response('not found', { status: 404 });
  }
}

Deno.serve(async (request) => {
  const response = await route(request, endpoints, store, (key) => Deno.env.get(key), config);
  return response ?? asset(new URL(request.url));
});
