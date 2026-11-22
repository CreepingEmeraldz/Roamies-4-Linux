// Which models a saved API key can run. Run: npm test
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { getModelsForProvider } from '@kenkaiiii/gg-core';
import { ALLOWED_PROVIDERS, createAllowed, norm } from '../allowed.mjs';
import { flushWrites } from '../store.mjs';

assert.equal(norm('models/gemini-2.5-pro'), 'gemini-2.5-pro');
assert.equal(norm('gpt-5-2025-08-07'), 'gpt-5');
assert.equal(norm('claude-sonnet-4-5-20250929'), 'claude-sonnet-4-5');
assert.equal(norm('gemini-flash-latest'), 'gemini-flash');
assert.ok(ALLOWED_PROVIDERS.includes('openai') && ALLOWED_PROVIDERS.includes('anthropic') && ALLOWED_PROVIDERS.includes('gemini'));

const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'roamies-allowed-'));
const known = getModelsForProvider('openai').map((m) => m.id);
assert.ok(known.length >= 2, 'gg-core lists OpenAI models');
let t = 1_000_000, key = { accessToken: 'sk-test' }, answer = null;
const calls = [];
const fakeFetch = async (url, init) => {
  calls.push({ url, headers: init.headers });
  if (!answer) throw new Error('offline');
  return { ok: answer.ok ?? true, status: answer.status ?? 200, json: async () => answer.body };
};
const auth = { getCredentials: async (k) => (k === 'openai' ? key : null) };
const make = () => createAllowed({ auth, dir, models: (p) => getModelsForProvider(p).map((m) => m.id), fetch: fakeFetch, now: () => t });

// Nothing known: everything is allowed.
let a = make();
await a.load();
assert.ok(known.every((id) => a.allows('openai', id)));

// The key can run only the first model: the others are hidden, and the key is sent as a bearer token.
answer = { body: { data: [{ id: known[0] }, { id: 'some-other-model' }] } };
// Only one of the known models appearing is not enough to trust the list when there are many.
const trusted = known.length <= 2;
const changed = await a.refresh('openai');
assert.equal(calls[0].url, 'https://api.openai.com/v1/models');
assert.equal(calls[0].headers.Authorization, 'Bearer sk-test');
if (trusted) { assert.equal(changed, true); assert.ok(a.allows('openai', known[0])); assert.ok(!a.allows('openai', known[1])); }
else assert.ok(known.every((id) => a.allows('openai', id)), 'a list that names few known models hides nothing');

// A list that names most of the known models hides the ones it leaves out, and dated names count.
const keep = known.slice(0, Math.ceil(known.length * 0.75));
const left = known.slice(keep.length);
answer = { body: { data: keep.map((id) => ({ id: `${id}-2025-08-07` })) } };
t += 7 * 3600_000;
a = make();
assert.equal(await a.refresh('openai'), true);
assert.ok(keep.every((id) => a.allows('openai', id)));
assert.ok(left.every((id) => !a.allows('openai', id)));
assert.ok(a.allows('anthropic', 'claude-anything'), 'other providers are untouched');

// A failed, empty or odd answer leaves the list as it was.
for (const bad of [{ ok: false, status: 500 }, { body: { data: [] } }, { body: { error: 'x' } }]) {
  answer = bad; t += 7 * 3600_000;
  assert.equal(await a.refresh('openai'), false);
  assert.ok(left.every((id) => !a.allows('openai', id)));
}
answer = null;
assert.equal(await a.refresh('openai'), false, 'offline');

// A restart offline: the saved list comes back.
await flushWrites();
const b = make();
await b.load();
assert.ok(left.every((id) => !b.allows('openai', id)));

// No key saved any more: nothing is hidden.
key = null; t += 7 * 3600_000;
assert.equal(await a.refresh('openai'), true);
assert.ok(known.every((id) => a.allows('openai', id)));

// Stale after six hours, not before.
key = { accessToken: 'sk-test' }; answer = { body: { data: known.map((id) => ({ id })) } };
calls.length = 0; t += 7 * 3600_000;
await a.refreshStale(['openai']); assert.equal(calls.length, 1);
await a.refreshStale(['openai']); assert.equal(calls.length, 1, 'not again right away');
await a.refreshStale(['gemini']); assert.equal(calls.length, 1, 'only the connected ones');

await fs.rm(dir, { recursive: true, force: true });
console.log('allowed ok');
