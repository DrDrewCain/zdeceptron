// The store, on Deno KV.
//
// Deno KV is the only one of the four backends with a literal `watch()`,
// and it has one limitation that shapes this file: **it takes an explicit
// key list, not a prefix.** A durable `Map` signal lives across as many
// keys as it has entries, so watching it directly is impossible.
//
// The fix is a version cell per signal, bumped inside the same atomic
// commit as every write to any of that signal's cells. `watch` then follows
// one key per signal — an explicit list the compiler already knows, because
// it knows every durable key in the program.
//
// Atomicity is compare-and-set rather than the native `sum` mutation.
// `sum` operates on `Deno.KvU64`: unsigned, 64-bit, and wrapping on
// overflow, which can represent neither a decrement below zero nor
// ZDeceptron's `Whole` (an f64, §14A.3). A versionstamp check with a
// bounded retry is atomic and does represent the type. `sum` is still used
// for the version cell, where unsigned and monotonic is exactly right.

import { address, appended, number, removed, replace, subkey } from './cells.js';

const kv = await Deno.openKv();
const PREFIX = 'zd';
const VERSION = '~version';
const ATTEMPTS = 8;

const cell = (key, sub) => [PREFIX, key, sub];
const version = (key) => [PREFIX, key, VERSION];

async function modify(key, args, produce) {
  const { sub, path, value } = address(args);
  for (let attempt = 0; attempt < ATTEMPTS; attempt += 1) {
    const entry = await kv.get(cell(key, sub));
    const next = replace(entry.value ?? null, path, (inner) => produce(inner, value));
    const result = await kv
      .atomic()
      .check(entry)
      .set(cell(key, sub), next)
      .mutate({ type: 'sum', key: version(key), value: new Deno.KvU64(1n) })
      .commit();
    if (result.ok) return next;
  }
  throw new Error(`the durable write to ${key} lost ${ATTEMPTS} times to concurrent writes`);
}

export const store = {
  async get(key, ...indices) {
    const sub = subkey(indices);
    if (sub !== '') return (await kv.get(cell(key, sub))).value ?? null;
    const out = {};
    let count = 0;
    let scalar = null;
    for await (const entry of kv.list({ prefix: [PREFIX, key] })) {
      const name = entry.key[entry.key.length - 1];
      if (name === VERSION) continue;
      count += 1;
      if (name === '') scalar = entry.value;
      else out[name] = entry.value;
    }
    if (count === 0) return null;
    if (count === 1 && scalar !== null) return scalar;
    if (scalar !== null) out[''] = scalar;
    return out;
  },

  set: (key, ...args) => modify(key, args, (_current, value) => value),
  append: (key, ...args) => modify(key, args, appended),
  remove: (key, ...args) => modify(key, args, removed),
  incr: (key, ...args) => modify(key, args, (current, by) => number(current) + number(by)),
  decr: (key, ...args) => modify(key, args, (current, by) => number(current) - number(by)),

  async watch(keys, emit) {
    const reader = kv.watch(keys.map(version)).getReader();
    const seen = new Map();
    let live = true;
    void (async () => {
      let first = true;
      while (live) {
        const { done, value: entries } = await reader.read();
        if (done) return;
        for (let index = 0; index < keys.length; index += 1) {
          const stamp = String(entries[index].versionstamp);
          if (seen.get(keys[index]) === stamp) continue;
          seen.set(keys[index], stamp);
          // The first delivery is the current state, which the router has
          // already sent as the resynchronisation frame.
          if (!first) emit(keys[index], await store.get(keys[index]));
        }
        first = false;
      }
    })();
    return () => {
      live = false;
      void reader.cancel();
    };
  },
};
