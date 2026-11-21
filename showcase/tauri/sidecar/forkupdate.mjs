// Update notices for this fork's Linux builds, without any signing: the newest release on the fork's GitHub page is
// compared with the build that is running, and a newer one is announced (the download page opens from the menu).
// Tags look like v0.1.46-linux.3; a plain v0.1.46 or v0.1.46-linux counts as build 0 of that version.

/** [major, minor, patch, build] from a tag or version ("v0.1.46-linux.3", "0.1.46"), or null. */
export function parseBuild(s) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)(?:-linux(?:\.(\d+))?)?$/.exec(String(s ?? '').trim());
  return m ? [+m[1], +m[2], +m[3], +(m[4] ?? 0)] : null;
}

/** Whether tag `a` is newer than `b` (both parsed or null). Something unreadable is never newer. */
export function isNewer(a, b) {
  const x = parseBuild(a), y = parseBuild(b);
  if (!x || !y) return false;
  for (let i = 0; i < 4; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}

/** The newest release of `repo` ({tag, url}), or throws. fetchImpl is fetch (a stand-in in tests). */
export async function latestRelease(repo, fetchImpl = fetch) {
  const res = await fetchImpl(`https://api.github.com/repos/${repo}/releases?per_page=30`, { headers: { accept: 'application/vnd.github+json', 'user-agent': 'roamies-update-check' }, signal: AbortSignal.timeout(15000) });
  if (!res.ok) throw new Error(`GitHub answered ${res.status}`);
  const list = await res.json();
  let best = null;
  for (const r of Array.isArray(list) ? list : []) {
    if (r?.draft || r?.prerelease || !parseBuild(r?.tag_name)) continue;
    if (!best || isNewer(r.tag_name, best.tag_name)) best = r;
  }
  if (!best) throw new Error('no release found');
  return { tag: String(best.tag_name).replace(/^v/, ''), url: String(best.html_url || '') };
}

/** { state: 'available', version } when the newest release is newer than `current`, else { state: 'none' }. */
export async function checkFork(repo, current, fetchImpl = fetch) {
  const r = await latestRelease(repo, fetchImpl);
  return isNewer(r.tag, current) ? { state: 'available', version: r.tag } : { state: 'none' };
}
