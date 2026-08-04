// The store, on a SQLite-backed Durable Object.
//
// A Durable Object is the only one of the four backends that is both the
// store and the push channel. Every storage method is implicitly wrapped in
// a transaction and input gates stop requests interleaving across an await,
// so a read-modify-write here is atomic without a compare-and-set loop —
// and the same single-threaded actor already holds the subscribers, so
// fan-out needs no second system.
//
// The object is addressed by one fixed name. An individual object has a
// documented soft ceiling of about 1,000 requests per second; shard by
// signal key before approaching it.

import { address, appended, at, number, removed, replace, subkey } from './cells.js';

const OBJECT = 'zd';

/** The worker-side client. Talks to the object over its own `fetch`. */
export function storeFor(env) {
  const stub = env.ZD_STORE.get(env.ZD_STORE.idFromName(OBJECT));
  const send = async (op, body) => {
    const response = await stub.fetch(`https://zd.invalid/${op}`, {
      method: 'POST',
      body: JSON.stringify(body),
    });
    if (!response.ok) throw new Error(`store ${op} failed: ${response.status}`);
    return response.json();
  };
  const write = (op) => (key, ...args) => send(op, { key, ...address(args) });

  return {
    get: (key, ...indices) => send('get', { key, sub: subkey(indices) }),
    set: write('set'),
    incr: write('incr'),
    decr: write('decr'),
    append: write('append'),
    remove: write('remove'),

    async watch(keys, emit) {
      const response = await stub.fetch(
        `https://zd.invalid/watch?keys=${encodeURIComponent(keys.join(','))}`,
      );
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      let live = true;
      void (async () => {
        while (live) {
          const { done, value } = await reader.read();
          if (done) return;
          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split('\n');
          buffer = lines.pop() ?? '';
          for (const line of lines) {
            if (line === '') continue;
            const change = JSON.parse(line);
            emit(change.key, change.value);
          }
        }
      })();
      return () => {
        live = false;
        void reader.cancel();
      };
    },
  };
}

/** The object itself. Named by `wrangler.toml`'s Durable Object binding. */
export class ZdStore {
  constructor(state) {
    this.storage = state.storage;
    this.subscribers = new Set();
  }

  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === '/watch') {
      return this.stream((url.searchParams.get('keys') || '').split(','));
    }
    const body = await request.json();
    const value = await this.apply(url.pathname.slice(1), body);
    return new Response(JSON.stringify(value === undefined ? null : value), {
      headers: { 'content-type': 'application/json' },
    });
  }

  cell(key, sub) {
    return sub === '' ? key : `${key} ${sub}`;
  }

  /** One signal's whole value: the scalar cell, or an object of subkeys. */
  async read(key) {
    const indexed = await this.storage.list({ prefix: `${key} ` });
    const scalar = await this.storage.get(key);
    if (indexed.size === 0) return scalar === undefined ? null : scalar;
    const out = {};
    if (scalar !== undefined) out[''] = scalar;
    for (const [name, value] of indexed) out[name.slice(key.length + 1)] = value;
    return out;
  }

  async apply(op, body) {
    if (op === 'get') {
      return body.sub === '' ? this.read(body.key) : this.storage.get(this.cell(body.key, body.sub));
    }
    const cell = this.cell(body.key, body.sub);
    const current = (await this.storage.get(cell)) ?? null;
    const produce = {
      set: () => body.value,
      incr: (old) => number(old) + number(body.value),
      decr: (old) => number(old) - number(body.value),
      append: (old) => appended(old, body.value),
      remove: (old) => removed(old, body.value),
    }[op];
    if (produce === undefined) throw new Error(`unknown store operation ${op}`);
    const next = replace(current, body.path, (inner) => produce(inner));
    await this.storage.put(cell, next);
    const whole = await this.read(body.key);
    for (const subscriber of this.subscribers) subscriber(body.key, whole);
    return at(next, body.path);
  }

  /** Newline-delimited JSON, so the worker side needs no framing parser. */
  stream(keys) {
    const wanted = new Set(keys);
    const encoder = new TextEncoder();
    let subscriber = null;
    const stream = new ReadableStream({
      start: (controller) => {
        subscriber = (key, value) => {
          if (!wanted.has(key)) return;
          try {
            controller.enqueue(encoder.encode(`${JSON.stringify({ key, value })}\n`));
          } catch {
            this.subscribers.delete(subscriber);
          }
        };
        this.subscribers.add(subscriber);
      },
      cancel: () => {
        if (subscriber !== null) this.subscribers.delete(subscriber);
      },
    });
    return new Response(stream, { headers: { 'content-type': 'application/x-ndjson' } });
  }
}
