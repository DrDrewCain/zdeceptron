// The store, on DynamoDB, over signed `fetch`.
//
// DynamoDB is a plain JSON-over-HTTP service, so the AWS SDK is not needed:
// SigV4 is an HMAC-SHA256 chain and Web Crypto is in ECMA-429. That keeps
// the deploy tree at zero npm dependencies — nothing to install, nothing to
// bundle, nothing to audit — which is worth the forty lines of signing.
//
// Atomicity: `SET n = if_not_exists(n, :zero) + :delta` is a documented
// atomic counter. AWS recommends this form over `ADD` because `ADD` is not
// idempotent under retry. `append` and `remove` are read-modify-write and
// are therefore last-writer-wins here; DynamoDB has no serialisation point
// to hide behind, unlike a Durable Object.
//
// There is no native watch. DynamoDB Streams are pull-based change capture
// with a hard ceiling of two readers per shard, which cannot back one
// stream per browser tab, so `store.watch` is deliberately absent and the
// router polls.

import { address, appended, number, removed, replace, subkey } from './cells.js';

const encoder = new TextEncoder();
const SERVICE = 'dynamodb';
const ALGORITHM = 'AWS4-HMAC-SHA256';

const region = () => process.env.ZD_REGION || process.env.AWS_REGION;
const table = () => process.env.ZD_TABLE;

function hex(bytes) {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

async function hmac(key, text) {
  const imported = await crypto.subtle.importKey(
    'raw',
    key,
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  return new Uint8Array(await crypto.subtle.sign('HMAC', imported, encoder.encode(text)));
}

async function sha256(text) {
  return hex(new Uint8Array(await crypto.subtle.digest('SHA-256', encoder.encode(text))));
}

async function call(target, body) {
  const payload = JSON.stringify(body);
  const stamp = new Date().toISOString().replace(/[:-]|\.\d{3}/g, '');
  const day = stamp.slice(0, 8);
  const host = `${SERVICE}.${region()}.amazonaws.com`;
  const scope = `${day}/${region()}/${SERVICE}/aws4_request`;

  // `host` is signed but not sent: `fetch` sets it itself, and setting it
  // explicitly is rejected as a forbidden header on some runtimes.
  const sent = {
    'content-type': 'application/x-amz-json-1.0',
    'x-amz-date': stamp,
    'x-amz-target': `DynamoDB_20120810.${target}`,
  };
  if (process.env.AWS_SESSION_TOKEN) sent['x-amz-security-token'] = process.env.AWS_SESSION_TOKEN;
  const signed = { ...sent, host };

  const names = Object.keys(signed).sort();
  const canonical = [
    'POST',
    '/',
    '',
    ...names.map((name) => `${name}:${signed[name]}`),
    '',
    names.join(';'),
    await sha256(payload),
  ].join('\n');

  let key = encoder.encode(`AWS4${process.env.AWS_SECRET_ACCESS_KEY}`);
  for (const part of [day, region(), SERVICE, 'aws4_request']) key = await hmac(key, part);
  const signature = hex(
    await hmac(key, [ALGORITHM, stamp, scope, await sha256(canonical)].join('\n')),
  );

  sent.authorization =
    `${ALGORITHM} Credential=${process.env.AWS_ACCESS_KEY_ID}/${scope}, ` +
    `SignedHeaders=${names.join(';')}, Signature=${signature}`;

  const response = await fetch(`https://${host}/`, { method: 'POST', headers: sent, body: payload });
  if (!response.ok) throw new Error(`DynamoDB ${target}: ${response.status} ${await response.text()}`);
  return response.json();
}

const decode = (item) =>
  item === undefined ? null : item.n !== undefined ? Number(item.n.N) : JSON.parse(item.j.S);

const encode = (value) =>
  typeof value === 'number' ? { n: { N: String(value) } } : { j: { S: JSON.stringify(value) } };

async function cell(key, sub) {
  const result = await call('GetItem', {
    TableName: table(),
    Key: { k: { S: key }, s: { S: sub } },
    ConsistentRead: true,
  });
  return decode(result.Item);
}

async function put(key, sub, value) {
  await call('PutItem', {
    TableName: table(),
    Item: { k: { S: key }, s: { S: sub }, ...encode(value) },
  });
}

/** The atomic path: one round trip, no read, no compare-and-set. */
async function add(key, sub, delta) {
  const result = await call('UpdateItem', {
    TableName: table(),
    Key: { k: { S: key }, s: { S: sub } },
    UpdateExpression: 'SET #n = if_not_exists(#n, :zero) + :delta REMOVE #j',
    ExpressionAttributeNames: { '#n': 'n', '#j': 'j' },
    ExpressionAttributeValues: { ':zero': { N: '0' }, ':delta': { N: String(delta) } },
    ReturnValues: 'UPDATED_NEW',
  });
  return Number(result.Attributes.n.N);
}

async function modify(key, args, produce) {
  const { sub, path, value } = address(args);
  const current = await cell(key, sub);
  const next = replace(current, path, (inner) => produce(inner, value));
  await put(key, sub, next);
  return next;
}

export const store = {
  async get(key, ...indices) {
    const sub = subkey(indices);
    if (sub !== '') return cell(key, sub);
    const result = await call('Query', {
      TableName: table(),
      KeyConditionExpression: '#k = :k',
      ExpressionAttributeNames: { '#k': 'k' },
      ExpressionAttributeValues: { ':k': { S: key } },
      ConsistentRead: true,
    });
    const items = result.Items ?? [];
    if (items.length === 0) return null;
    if (items.length === 1 && items[0].s.S === '') return decode(items[0]);
    const out = {};
    for (const item of items) out[item.s.S] = decode(item);
    return out;
  },

  set: (key, ...args) => modify(key, args, (_current, value) => value),
  append: (key, ...args) => modify(key, args, appended),
  remove: (key, ...args) => modify(key, args, removed),

  incr(key, ...args) {
    const { sub, path, value } = address(args);
    if (path.length === 0) return add(key, sub, number(value));
    return modify(key, args, (current, by) => number(current) + number(by));
  },

  decr(key, ...args) {
    const { sub, path, value } = address(args);
    if (path.length === 0) return add(key, sub, -number(value));
    return modify(key, args, (current, by) => number(current) - number(by));
  },
};
