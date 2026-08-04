// Vercel Functions entry.
//
// Vercel's `fetch` Web Standard export is the same shape Cloudflare and
// Deno use, so this shim is the shortest of the four. The static half of
// the bundle is served from `public/` by Vercel's zero-config static
// handling, and `vercel.json` rewrites `/_zd/*` here.
//
// `export const config` is Vercel's per-function configuration and is not
// the `_zd/config.js` this file also imports, which is why that one is
// renamed on the way in.

import { endpoints } from '../_zd/endpoints.js';
import { config as zd } from '../_zd/config.js';
import { json, route } from '../_zd/router.js';
import { store } from '../_zd/store.js';

export default {
  async fetch(request) {
    const response = await route(request, endpoints, store, (key) => process.env[key], zd);
    return response ?? json({ error: 'not found' }, 404);
  },
};
