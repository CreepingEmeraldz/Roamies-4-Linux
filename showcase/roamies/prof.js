// Diagnostic, off unless ROAMIES_PROFILE=1: every 10 s, what the page spent its time on, sent to ~/.../profile.log.
(() => {
  if (!window.ROAMIE_PROF) return;
  const T = new Map(), N = new Map(), INV = new Map(), MUT = new Map(), CV = {};
  let raf = 0, efp = 0, reports = 0;
  const label = (f) => (f && f.name) || String(f).replace(/\s+/g, ' ').slice(0, 80);
  const timed = (kind, f) => {
    if (typeof f !== 'function') return f;
    const l = kind + ':' + label(f);
    return function (...a) {
      const t = performance.now();
      try { return f.apply(this, a); } finally { T.set(l, (T.get(l) || 0) + performance.now() - t); N.set(l, (N.get(l) || 0) + 1); }
    };
  };
  const st = window.setTimeout, si = window.setInterval, rf = window.requestAnimationFrame;
  window.setTimeout = (f, t, ...a) => st(timed('timeout', f), t, ...a);
  window.setInterval = (f, t, ...a) => si(timed('interval', f), t, ...a);
  window.requestAnimationFrame = (f) => { raf++; return rf(timed('raf', f)); };
  const ctx = window.CanvasRenderingContext2D?.prototype;
  for (const m of ['clearRect', 'fillRect', 'drawImage', 'fillText', 'putImageData', 'stroke', 'fill']) {
    const o = ctx?.[m]; if (!o) continue;
    ctx[m] = function (...a) { CV[m] = (CV[m] || 0) + 1; return o.apply(this, a); };
  }
  const ef = document.elementFromPoint.bind(document); document.elementFromPoint = (x, y) => { efp++; return ef(x, y); };
  const efs = document.elementsFromPoint.bind(document); document.elementsFromPoint = (x, y) => { efp++; return efs(x, y); };
  const wrapInvoke = () => {
    const c = window.__TAURI__?.core; if (!c || c.__p) return;
    const o = c.invoke; c.__p = 1; c.invoke = (cmd, ...r) => { INV.set(cmd, (INV.get(cmd) || 0) + 1); return o(cmd, ...r); };
  };
  wrapInvoke();
  new MutationObserver((recs) => {
    for (const r of recs) {
      const t = r.target, k = (t.nodeName || '?') + (t.id ? '#' + t.id : '') + (t.className && typeof t.className === 'string' ? '.' + t.className.split(' ')[0] : '') + ' ' + r.type + (r.attributeName ? ':' + r.attributeName : '');
      MUT.set(k, (MUT.get(k) || 0) + 1);
    }
  }).observe(document.documentElement, { subtree: true, childList: true, attributes: true, characterData: true });
  const top = (m, n, f = (v) => v) => [...m.entries()].sort((a, b) => b[1] - a[1]).slice(0, n).map(([k, v]) => k + '=' + f(v)).join(' | ');
  let wakes0 = 0;
  si(() => {
    wrapInvoke();
    const R = window.Roamie || {};
    const line = {
      type: 'prof', n: ++reports, quiet_s: Math.round((performance.now() - (R.lastActive || 0)) / 1000), wakes: (R.wakes || 0) - wakes0, raf, elementFromPoint: efp,
      jsms: Math.round([...T.values()].reduce((a, b) => a + b, 0)), time: top(T, 6, (v) => Math.round(v) + 'ms'), calls: top(N, 6), invokes: top(INV, 6), mutations: top(MUT, 6), canvas: JSON.stringify(CV),
    };
    wakes0 = R.wakes || 0; raf = efp = 0; T.clear(); N.clear(); INV.clear(); MUT.clear(); for (const k in CV) delete CV[k];
    if (reports <= 30) window.__TAURI__?.core?.invoke?.('sidecar_send', { msg: line });
  }, 10000);
})();
