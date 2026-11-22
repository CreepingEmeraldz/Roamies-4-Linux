// OpenRouter's and OpenCode's model lists. gg-core ships only a few rows for each, so the Model menu showed just
// those; this reads the live lists and caches them so they survive restarts and offline starts.
import path from 'node:path';
import {
  OPENCODE_CATALOG_URLS, OPENROUTER_DEFAULT_BASE_URL, getProviderCatalog, parseOpenCodeCatalog, parseOpenRouterCatalog, setProviderCatalog,
} from '@kenkaiiii/gg-core';
import { readJson, writeJson } from './store.mjs';

export const CATALOG_PROVIDERS = ['openrouter', 'opencode', 'opencode-go'];
export const REFRESH_MS = 6 * 3600_000;
export const RETRY_MS = 10 * 60_000;

const OPENROUTER_FIELDS = ['id', 'name', 'context_length', 'supported_parameters', 'architecture', 'reasoning', 'top_provider', 'pricing'];
const parse = (p, body) => (p === 'openrouter' ? parseOpenRouterCatalog(body) : parseOpenCodeCatalog(p, body)).models;

/** → { load, refresh(provider), refreshStale(connected) }; the last two resolve true when the menu's list changed. */
export function createCatalogs({ auth, dir, fetch: doFetch = (...a) => fetch(...a), now = Date.now, timeoutMs = 15_000 }) {
  const nextRead = new Map(), inFlight = new Map();
  const file = (p) => path.join(dir, `${p}-models.json`);

  // Where the list comes from: OpenRouter's is the key's own endpoint (a custom base URL lists its own models). With a
  // key, /models/user is asked first: it leaves out the models the account's privacy settings, provider preferences and
  // guardrails rule out. If that isn't answered, the full /models list is used (`plain`).
  async function source(p) {
    if (p !== 'openrouter') return { url: OPENCODE_CATALOG_URLS[p], headers: {} };
    const creds = await auth.getCredentials('openrouter').catch(() => null);
    const base = (creds?.baseUrl ?? OPENROUTER_DEFAULT_BASE_URL).replace(/\/+$/, '');
    const keyed = !!creds?.accessToken;
    return { url: keyed ? `${base}/models/user` : `${base}/models`, plain: `${base}/models`, headers: keyed ? { Authorization: `Bearer ${creds.accessToken}` } : {} };
  }

  function install(p, body) {
    const models = parse(p, body);
    // An empty answer never wipes a working list.
    if (!models.length) return false;
    const before = JSON.stringify(getProviderCatalog(p) ?? null);
    return JSON.stringify(setProviderCatalog(p, models)) !== before;
  }

  async function load() {
    for (const p of CATALOG_PROVIDERS) {
      const cached = readJson(file(p));
      if (!cached?.body || cached.url !== (await source(p)).url) continue;
      install(p, cached.body);
      nextRead.set(p, (Number(cached.syncedAt) || 0) + REFRESH_MS);
    }
  }

  async function read(p) {
    nextRead.set(p, now() + RETRY_MS);
    try {
      const { url, plain, headers } = await source(p);
      const get = async (u) => {
        const res = await doFetch(u, { headers: { Accept: 'application/json', ...headers }, signal: AbortSignal.timeout(timeoutMs) });
        if (!res.ok) return null;
        const b = await res.json();
        return parse(p, b).length ? b : null;
      };
      let body = await get(url).catch(() => null);
      if (!body && plain && plain !== url) body = await get(plain);
      if (!body) return false;
      // OpenRouter's answer is ~1 MB: keep only the rows that can call tools, and only the fields gg-core reads.
      if (p === 'openrouter') {
        const keep = new Set(parse(p, body).map((m) => m.id));
        body = { data: body.data.filter((r) => keep.has(r?.id?.trim?.())).map((r) => Object.fromEntries(OPENROUTER_FIELDS.filter((k) => k in r).map((k) => [k, r[k]]))) };
      }
      await writeJson(file(p), { url, syncedAt: now(), body });
      nextRead.set(p, now() + REFRESH_MS);
      return install(p, body);
    } catch {
      return false;
    }
  }

  const refresh = (p) => {
    let running = inFlight.get(p);
    if (!running) { running = read(p).finally(() => inFlight.delete(p)); inFlight.set(p, running); }
    return running;
  };

  const refreshStale = async (connected) => (await Promise.all(CATALOG_PROVIDERS
    .filter((p) => connected.includes(p) && (inFlight.has(p) || now() >= (nextRead.get(p) ?? 0)))
    .map(refresh))).some(Boolean);

  return { load, refresh, refreshStale };
}
