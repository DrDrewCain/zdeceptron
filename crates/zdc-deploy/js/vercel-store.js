// The store, on Upstash Redis over its REST API.
//
// Vercel has no first-party store that can do this. Vercel KV and Vercel
// Postgres no longer exist as products — Postgres moved to Neon, KV was
// sunset when Upstash joined the Marketplace — and the only first-party
// key-value surface left, Global Config, is a 1 MB read-only config channel
// with up to ten seconds of propagation delay. So the adapter targets the
// Marketplace provider the documentation itself points at, and generates no
// configuration for products that are gone.
//
// One Redis hash per durable signal, one field per subkey. `HINCRBYFLOAT`
// is the atomic counter; `append` and `remove` are read-modify-write and
// therefore last-writer-wins.
//
// There is no watch. Upstash's REST API does expose `SUBSCRIBE` over SSE,
// but consuming it would mean holding a second streamed connection open
// inside the same `maxDuration` budget as the one being served. The router
// polls instead, which costs a round trip per interval and cannot be cut
// off mid-stream by the function timing out.

import { address, appended, number, removed, replace, subkey } from './cells.js';

const base = () => process.env.UPSTASH_REDIS_REST_URL;
const token = () => process.env.UPSTASH_REDIS_REST_TOKEN;

async function command(...parts) {
  const response = await fetch(base(), {
    method: 'POST',
    headers: { authorization: `Bearer ${token()}`, 'content-type': 'application/json' },
    body: JSON.stringify(parts.map(String)),
  });
  if (!response.ok) throw new Error(`Upstash ${parts[0]}: ${response.status}`);
  const body = await response.json();
  if (body.error) throw new Error(`Upstash ${parts[0]}: ${body.error}`);
  return body.result;
}

const hash = (key) => `zd:${key}`;
const parse = (text) => (text === null || text === undefined ? null : JSON.parse(text));

async function modify(key, args, produce) {
  const { sub, path, value } = address(args);
  const current = parse(await command('HGET', hash(key), sub));
  const next = replace(current, path, (inner) => produce(inner, value));
  await command('HSET', hash(key), sub, JSON.stringify(next));
  return next;
}

export const store = {
  async get(key, ...indices) {
    const sub = subkey(indices);
    if (sub !== '') return parse(await command('HGET', hash(key), sub));
    const flat = (await command('HGETALL', hash(key))) ?? [];
    if (flat.length === 0) return null;
    if (flat.length === 2 && flat[0] === '') return parse(flat[1]);
    const out = {};
    for (let index = 0; index < flat.length; index += 2) out[flat[index]] = parse(flat[index + 1]);
    return out;
  },

  set: (key, ...args) => modify(key, args, (_current, value) => value),
  append: (key, ...args) => modify(key, args, appended),
  remove: (key, ...args) => modify(key, args, removed),

  async incr(key, ...args) {
    const { sub, path, value } = address(args);
    if (path.length > 0) return modify(key, args, (current, by) => number(current) + number(by));
    return number(await command('HINCRBYFLOAT', hash(key), sub, number(value)));
  },

  async decr(key, ...args) {
    const { sub, path, value } = address(args);
    if (path.length > 0) return modify(key, args, (current, by) => number(current) - number(by));
    return number(await command('HINCRBYFLOAT', hash(key), sub, -number(value)));
  },
};
