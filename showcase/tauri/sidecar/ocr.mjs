// Finding words on the screen without a helper (Linux): the screenshot is read with tesseract, and a phrase is looked
// up among the words it found. The boxes come back in the picture's pixels; tools.mjs turns them into screen points.
import { spawn } from 'node:child_process';

/** tesseract's TSV output → words: { text, x, y, w, h, line } (a line is a block/paragraph/line number triple). */
export function parseTsv(tsv) {
  const words = [];
  for (const row of String(tsv).split('\n').slice(1)) {
    const c = row.split('\t');
    if (c.length < 12 || c[0] !== '5') continue;
    const text = c.slice(11).join('\t').trim();
    const conf = Number(c[10]);
    if (!text || conf < 20) continue;
    const [x, y, w, h] = c.slice(6, 10).map(Number);
    if (![x, y, w, h].every(Number.isFinite) || w <= 0 || h <= 0) continue;
    words.push({ text, x, y, w, h, line: `${c[2]}.${c[3]}.${c[4]}` });
  }
  return words;
}

const norm = (s) => String(s).toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '');

/** Every place `find` (one word or a phrase) is read on screen, in reading order: { text, bounds: [x, y, w, h] } in the
 *  picture's pixels. A phrase has to be consecutive words on one line; a single word may sit inside a longer one
 *  ("Search" in "Search…") only when it is a few letters or more. A single character has to match exactly. */
export function findWords(words, find) {
  const q = String(find).trim();
  const toks = q.split(/\s+/).map(norm).filter(Boolean);
  if (!toks.length) return [];
  const single = q.length === 1;
  const out = [];
  for (let i = 0; i + toks.length <= words.length; i++) {
    const run = words.slice(i, i + toks.length);
    if (run.some((w) => w.line !== run[0].line)) continue;
    const ok = run.every((w, k) => {
      const n = norm(w.text);
      if (single) return w.text.trim() === q;
      return n === toks[k] || (toks.length === 1 && toks[k].length >= 3 && n.length <= toks[k].length + 4 && n.includes(toks[k]));
    });
    if (!ok) continue;
    const x0 = Math.min(...run.map((w) => w.x)), y0 = Math.min(...run.map((w) => w.y));
    const x1 = Math.max(...run.map((w) => w.x + w.w)), y1 = Math.max(...run.map((w) => w.y + w.h));
    out.push({ text: run.map((w) => w.text).join(' '), bounds: [x0, y0, x1 - x0, y1 - y0] });
    i += toks.length - 1;
  }
  return out;
}

/** The words tesseract reads in an image file. Rejects with `missing: true` when tesseract isn't installed. */
export function readWords(file, { timeout = 30000 } = {}) {
  return new Promise((resolve, reject) => {
    const p = spawn('tesseract', [file, 'stdout', '--psm', '11', '-l', 'eng', '-c', 'thresholding_method=1', 'tsv'], {
      stdio: ['ignore', 'pipe', 'ignore'],
      // One thread: tesseract's own threads fight over the cores and, on a many-core laptop, take several times longer.
      env: { ...process.env, OMP_THREAD_LIMIT: '1' },
    });
    let out = '', done = false;
    const end = (fn, v) => { if (done) return; done = true; clearTimeout(t); fn(v); };
    const t = setTimeout(() => { p.kill('SIGKILL'); end(reject, new Error('Reading the screen took too long.')); }, timeout);
    p.stdout.on('data', (d) => { out += d; });
    p.on('error', (e) => end(reject, Object.assign(new Error(e.code === 'ENOENT' ? 'tesseract is not installed' : e.message), { missing: e.code === 'ENOENT' })));
    p.on('close', (code) => (code === 0 ? end(resolve, parseTsv(out)) : end(reject, new Error(`tesseract stopped (${code})`))));
  });
}

/** A PNG's pixel size, from its header. */
export function pngSize(buf) {
  return buf.length > 24 && buf.toString('latin1', 1, 4) === 'PNG' ? [buf.readUInt32BE(16), buf.readUInt32BE(20)] : null;
}
