// The Commands page's rules: when a shell command needs a yes, which kinds are blocked outright, and the commands the
// user saved for Roamies to run by name. Pure functions (test/commands.test.mjs). These only ever add to the built-in
// safety rules in safety.mjs: a command that is blocked there stays blocked, whatever is set here.
import { splitCommand } from './safety.mjs';

export const MODES = ['smart', 'always', 'never', 'listed'];
export const SAVED_ASK = ['default', 'always', 'never'];
const MAX_LIST = 100, MAX_SAVED = 50;

export const DEFAULTS = { mode: 'smart', instant: false, blocked: [], trusted: [], saved: [] };

const text = (v, n) => (typeof v === 'string' ? v.replace(/[\r\n\t]+/g, ' ').trim().slice(0, n) : '');
const list = (v) => [...new Set((Array.isArray(v) ? v : []).map((x) => text(x, 120)).filter(Boolean))].slice(0, MAX_LIST);

/** A saved command as stored: trimmed, bounded, with an id made from its name. */
export function cleanSaved(c) {
  const name = text(c?.name, 40), command = text(c?.command, 500);
  if (!name || !command) return null;
  const phrases = [...new Set((Array.isArray(c?.phrases) ? c.phrases : typeof c?.phrases === 'string' ? c.phrases.split(/[,\n]/) : []).map((p) => text(p, 80)).filter(Boolean))].slice(0, 8);
  return { id: name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'command', name, phrases, command, ask: SAVED_ASK.includes(c?.ask) ? c.ask : 'default', instant: c?.instant !== false };
}

/** Whatever was stored (or sent from the panel), made safe to use. */
export function normalize(raw) {
  const seen = new Set(), saved = [];
  for (const c of Array.isArray(raw?.saved) ? raw.saved : []) {
    const s = cleanSaved(c);
    if (s && !seen.has(s.name.toLowerCase()) && saved.length < MAX_SAVED) { seen.add(s.name.toLowerCase()); saved.push(s); }
  }
  return { mode: MODES.includes(raw?.mode) ? raw.mode : 'smart', instant: raw?.instant === true, blocked: list(raw?.blocked), trusted: list(raw?.trusted), saved };
}

/** A pattern with * is a wildcard match on the whole command; without one it matches that command and any with arguments after it ("git status" also matches "git status -s"). */
export function matches(pattern, cmd) {
  const p = String(pattern ?? '').trim(), c = String(cmd ?? '').trim();
  if (!p || !c) return false;
  if (p.includes('*')) return new RegExp(`^${p.split('*').map((x) => x.replace(/[.+?^${}()|[\]\\]/g, '\\$&')).join('.*')}$`, 's').test(c);
  return c === p || c.startsWith(`${p} `);
}

// The separate commands in a line ("a && b | c"), so a pattern can't be dodged by chaining.
const partsOf = (cmd) => { const p = splitCommand(String(cmd ?? '').trim()); return p?.length ? p.map((x) => x.text.trim()).filter(Boolean) : [String(cmd ?? '').trim()]; };

/**
 * What to do with a command the built-in rules didn't block. level: 'safe' | 'ask' from safety.mjs.
 * → { action: 'run' | 'ask' | 'block', reason, forced }. forced: it asks because the user said so here, so "Skip Allow prompts" doesn't
 * skip it. askSetting: a saved command's own "ask" choice.
 */
export function decide(cmd, level, policy = DEFAULTS, askSetting = 'default') {
  const p = normalize(policy), parts = partsOf(cmd);
  if (parts.some((x) => p.blocked.some((b) => matches(b, x)))) return { action: 'block', reason: 'You blocked this kind of command on the Commands page.' };
  if (askSetting === 'always') return { action: 'ask', reason: '', forced: true };
  if (askSetting === 'never') return { action: 'run', reason: '' };
  switch (p.mode) {
    case 'always': return { action: 'ask', reason: '', forced: true };
    case 'never': return { action: 'run', reason: '' };
    case 'listed': return parts.length && parts.every((x) => p.trusted.some((t) => matches(t, x))) ? { action: 'run', reason: '' } : { action: 'ask', reason: '', forced: true };
    default: return { action: level === 'ask' ? 'ask' : 'run', reason: '', forced: false };
  }
}

/** The saved command a name points at (the exact name first, then an id or a phrase, ignoring case). */
export function findSaved(saved, name) {
  const n = String(name ?? '').trim().toLowerCase();
  if (!n) return null;
  const all = Array.isArray(saved) ? saved : [];
  return all.find((c) => c.name.toLowerCase() === n) ?? all.find((c) => c.id === n || c.phrases.some((p) => p.toLowerCase() === n)) ?? null;
}

// Words only: capitals, punctuation and spacing don't matter ("Next window." is "next window").
export const words = (t) => String(t ?? '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();

/**
 * The saved command a message IS (when instant commands are on): the whole message has to be its name or one of its
 * phrases, so "put it next window, not this window" is not "next window". null for anything else.
 */
export function instantMatch(policy, text) {
  const p = normalize(policy), w = words(text);
  if (!p.instant || !w) return null;
  return p.saved.find((c) => c.instant && [c.name, ...c.phrases].some((x) => words(x) === w)) ?? null;
}

/** The prompt's line about saved commands ('' when there are none). */
export function savedLine(saved) {
  if (!saved?.length) return '';
  return `- Saved commands (the user made these; run one with run_saved_command, by name, when they ask for it in those words or close to them):\n${saved.map((c) => `  - ${c.name}${c.phrases.length ? ` (${c.phrases.map((p) => `"${p}"`).join(', ')})` : ''}`).join('\n')}\n`;
}
