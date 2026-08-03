// Cloudflare Workers entry. A module worker, which is the only entrypoint
// shape ECMA-429 does not define and every platform therefore spells
// differently.
//
// This is the strongest of the four targets: there is no documented hard
// duration limit on an HTTP-triggered Worker, and billing is on CPU time
// rather than wall clock, so a parked SSE connection is close to free.

import { route } from './_zd/router.js';
import { endpoints } from './_zd/endpoints.js';
import { config } from './_zd/config.js';
import { storeFor, ZdStore } from './_zd/store.js';

// The Durable Object class has to be exported from the worker's own module
// for `wrangler.toml`'s binding to find it.
export { ZdStore };

export default {
  async fetch(request, env) {
    const response = await route(request, endpoints, storeFor(env), (key) => env[key], config);
    // `null` means the path was not an endpoint, so it is a static asset.
    return response === null ? env.ASSETS.fetch(request) : response;
  },
};
