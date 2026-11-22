// Which models an API key can actually use. gg-core lists every model a provider has; a key often can't call all of them
// (a new account, a tier, a plan), and picking one fails when it's asked. For the providers whose model list answers to
// the key, this asks it and keeps the answer, so the Model menu can leave out what the key can't run.
// It only ever hides models: a failed or odd-looking answer leaves the menu as it was.
import path from 'node:path';
import { apiKeyStorageKeys } from '@kenkaiiii/gg-core';
import { readJson, writeJson } from './store.mjs';

export const REFRESH_MS = 6 * 3600_000;
export const RETRY_MS = 10 * 60_000;

const bearer = (k) => ({ Authorization: `Bearer ${k}` });
// url: the list; ids: the model names in the answer; base: a saved custom base URL replaces the default one.
export const LISTS = {
  openai: { url: 'https://api.openai.com/v1/models', headers: bearer, ids: (b) => b.data?.map((r) => r.id) },
  anthropic: { url: 'https://api.anthropic.com/v1/models?limit=1000', headers: (k) => ({ 'x-api-key': k, 'anthropic-version': '2023-06-01' }), ids: (b) => b.data?.map((r) => r.id) },
  gemini: { url: 'https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000', headers: (k) => ({ 'x-goog-api-key': k }), ids: (b) => b.models?.map((r) => r.name) },
  xai: { url: 'https://api.x.ai/v1/models', headers: bearer, ids: (b) => b.data?.map((r) => r.id) },
  deepseek: { url: 'https://api.deepseek.com/models', headers: bearer, ids: (b) => b.data?.map((r) => r.id) },
  moonshot: { url: 'https://api.moonshot.ai/v1/models', headers: bearer, ids: (b) => b.data?.map((r) => r.id), base: true },
};
export const ALLOWED_PROVIDERS = Object.keys(LISTS);

// "models/gemini-2.5-pro", "gpt-5-2025-08-07", "claude-sonnet-4-5-20250929" and "...-latest" are one model each.
export const norm = (id) => String(id ?? '').trim().toLowerCase().replace(/^models\//, '')
  .replace(/-latest$/, '').replace(/-\d{4}-\d{2}-\d{2}$/, '').replace(/-\d{8}$/, '');

/** → { load, refresh(p), refreshStale(connected), allows(p, id) }; the refreshes resolve true when a list changed. */
export function createAllowed({ auth, dir, models, fetch: doFetch = (...a) => fetch(...a), now = Date.now, timeoutMs = 15_000 }) {
  const lists = new Map(), nextRead = new Map(), inFlight = new Map();
  const file = (p) => path.join(dir, `${p}-allowed.json`);

  // The key's list only counts when it plainly speaks about the same models gg-core knows: if under half of those
  // appear in it (other names, a proxy, a custom endpoint), nothing is hidden.
  function set(p, ids) {
    const have = new Set(ids.map(norm).filter(Boolean));
    const known = models(p).map(norm);
    const hit = known.filter((id) => have.has(id)).length;
    const next = known.length && hit * 2 >= known.length ? have : null;
    const same = JSON.stringify([...(lists.get(p) ?? [])].sort()) === JSON.stringify([...(next ?? [])].sort());
    if (next) lists.set(p, next); else lists.delete(p);
    return !same;
  }

  async function key(p) {
    for (const k of apiKeyStorageKeys(p)) {
      const c = await auth.getCredentials(k).catch(() => null);
      if (c?.accessToken) return c;
    }
    return null;
  }

  async function load() {
    for (const p of ALLOWED_PROVIDERS) {
      const c = readJson(file(p));
      if (Array.isArray(c?.ids)) { set(p, c.ids); nextRead.set(p, (Number(c.syncedAt) || 0) + REFRESH_MS); }
    }
  }

  async function read(p) {
    nextRead.set(p, now() + RETRY_MS);
    const spec = LISTS[p];
    try {
      const c = await key(p);
      if (!c) { const had = lists.has(p); lists.delete(p); return had; }
      const url = spec.base && c.baseUrl ? `${c.baseUrl.replace(/\/+$/, '')}/models` : spec.url;
      const res = await doFetch(url, { headers: { Accept: 'application/json', ...spec.headers(c.accessToken) }, signal: AbortSignal.timeout(timeoutMs) });
      if (!res.ok) return false;
      const ids = spec.ids(await res.json());
      if (!Array.isArray(ids) || !ids.length) return false;
      await writeJson(file(p), { syncedAt: now(), ids });
      nextRead.set(p, now() + REFRESH_MS);
      return set(p, ids);
    } catch {
      return false;
    }
  }

  const refresh = (p) => {
    if (!LISTS[p]) return Promise.resolve(false);
    let running = inFlight.get(p);
    if (!running) { running = read(p).finally(() => inFlight.delete(p)); inFlight.set(p, running); }
    return running;
  };
  const refreshStale = async (connected) => (await Promise.all(ALLOWED_PROVIDERS
    .filter((p) => connected.includes(p) && (inFlight.has(p) || now() >= (nextRead.get(p) ?? 0)))
    .map(refresh))).some(Boolean);

  const allows = (p, id) => { const l = lists.get(p); return !l || l.has(norm(id)); };
  return { load, refresh, refreshStale, allows };
}
