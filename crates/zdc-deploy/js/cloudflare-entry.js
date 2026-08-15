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
import { schedule } from './_zd/schedule.js';
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

  // §14G.4's scheduled jobs. A cron invocation is not a request: there is
  // no `Request`, no URL and no response, which is exactly why a job is not
  // an endpoint and is absent from the table above.
  //
  // `controller.cron` is the rule that fired, and a worker may hold
  // several, so the jobs are filtered by it — an hourly job must not run on
  // the minutely job's beat. `controller.scheduledTime` is when the beat
  // was *due* rather than when the platform got to it, which is what makes
  // a skipped beat observable as a jump larger than the cadence.
  //
  // `$store` and `$env` are installed here rather than by the router,
  // because the router is the request path and this is not one. The
  // emitted job bodies reference both as free identifiers under §8.2's
  // injection contract.
  async scheduled(controller, env, ctx) {
    globalThis.$store = storeFor(env);
    globalThis.$env = (key) => env[key];
    const at = Math.floor(controller.scheduledTime / 1000);
    for (const job of schedule) {
      if (job.cron !== controller.cron) continue;
      // `waitUntil`, so a job that outlives the handler's own promise is
      // still finished rather than cancelled when `scheduled` returns.
      ctx.waitUntil(job.handler({ [job.input]: at }));
    }
  },
};
