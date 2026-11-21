// What Roamies needs from a Linux desktop, checked for the Dependencies page: each item says whether it's there, what it
// is for, and how to get it. Pure where it can be: the checks run through `which` and `run` so tests can stand in.
import { spawn } from 'node:child_process';

/** The package manager's install line for this distro (from /etc/os-release's ID and ID_LIKE), for `names` by manager. */
export function installLine(osRelease, names) {
  const id = (/^ID=("?)([^"\n]*)\1$/m.exec(osRelease ?? '') ?? [])[2] ?? '';
  const like = (/^ID_LIKE=("?)([^"\n]*)\1$/m.exec(osRelease ?? '') ?? [])[2] ?? '';
  const fam = `${id} ${like}`.toLowerCase().split(/\s+/);
  const pick = (m) => names[m]?.join(' ');
  if (fam.some((f) => ['debian', 'ubuntu', 'zorin', 'mint', 'pop'].includes(f))) return pick('apt') && `sudo apt install ${pick('apt')}`;
  if (fam.some((f) => ['fedora', 'rhel', 'centos'].includes(f))) return pick('dnf') && `sudo dnf install ${pick('dnf')}`;
  if (fam.some((f) => ['arch', 'manjaro', 'endeavouros'].includes(f))) return pick('pacman') && `sudo pacman -S ${pick('pacman')}`;
  if (fam.some((f) => ['suse', 'opensuse'].includes(f))) return pick('zypper') && `sudo zypper install ${pick('zypper')}`;
  return null;
}

/** The screenshot portal's backend for this desktop, as package names by manager. */
export function portalPackages(desktop = '') {
  const d = String(desktop).toLowerCase();
  const name = d.includes('hypr') ? 'xdg-desktop-portal-hyprland' : /sway|river|wlroots|labwc/.test(d) ? 'xdg-desktop-portal-wlr' : d.includes('kde') ? 'xdg-desktop-portal-kde' : 'xdg-desktop-portal-gnome';
  return { apt: [name], dnf: [name], pacman: [name], zypper: [name] };
}

export const runOut = (cmd, args, timeout = 4000) => new Promise((resolve) => {
  let out = '';
  const p = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'ignore'] });
  const t = setTimeout(() => { p.kill('SIGKILL'); resolve(null); }, timeout);
  p.stdout.on('data', (d) => { out += d; });
  p.on('error', () => { clearTimeout(t); resolve(null); });
  p.on('close', (code) => { clearTimeout(t); resolve(code === 0 ? out : null); });
});

/** [{ id, name, ok, what, fix?, note? }]. env: { overlay: 'layer'|'xwayland'|'x11', session, desktop, osRelease }; run(cmd, args) → stdout or null. */
export async function checkDeps({ overlay, session, desktop, osRelease, run = runOut }) {
  const has = async (cmd) => (await run('sh', ['-c', `command -v ${cmd}`])) != null;
  const out = [];

  const langs = await run('tesseract', ['--list-langs']);
  const tessOk = langs != null && /^eng$/m.test(langs);
  out.push({
    id: 'tesseract', name: 'Tesseract (reads words on screen)', ok: tessOk,
    what: 'Lets Roamies find the exact box of a button, link or label it points at, instead of guessing from pixels.',
    ...(tessOk ? {} : { fix: installLine(osRelease, { apt: ['tesseract-ocr'], dnf: ['tesseract'], pacman: ['tesseract', 'tesseract-data-eng'], zypper: ['tesseract-ocr'] }) ?? 'Install the tesseract package (with its English data) from your distro.', note: langs != null ? 'It is installed but has no English language data.' : null }),
  });

  const gd = await has('gdbus');
  out.push({
    id: 'gdbus', name: 'gdbus (fast shortcut helper)', ok: gd,
    what: 'roamies-ctl uses it to tell the running Roamies what to do in a few milliseconds when you press a bound key.',
    ...(gd ? {} : { fix: installLine(osRelease, { apt: ['libglib2.0-bin'], dnf: ['glib2'], pacman: ['glib2'], zypper: ['glib2-tools'] }) ?? 'Install glib\'s command line tools (gdbus).' }),
  });

  if (session === 'wayland') {
    const portal = await run('gdbus', ['call', '--session', '--dest', 'org.freedesktop.DBus', '--object-path', '/org/freedesktop/DBus', '--method', 'org.freedesktop.DBus.NameHasOwner', 'org.freedesktop.portal.Desktop']);
    const ok = portal != null && /true/.test(portal);
    out.push({
      id: 'portal', name: 'Screenshot portal', ok,
      what: 'On Wayland, Roamies can only see the screen through your desktop\'s screenshot portal.',
      ...(ok ? {} : { fix: installLine(osRelease, portalPackages(desktop)) ?? 'Install xdg-desktop-portal and the backend for your desktop.' }),
    });
  }

  if (session === 'wayland') {
    const layer = overlay === 'layer';
    out.push({
      id: 'layer', name: 'Layer-shell (overlay above everything)', ok: layer,
      what: 'Lets the overlay sit above all windows and let clicks through. niri, sway, Hyprland and river have it; GNOME and KDE do not.',
      ...(layer ? {} : { note: 'Not in use: your desktop does not offer it (or ROAMIES_LAYER=0 is set), so Roamies uses its XWayland overlay. Drawing works, but it can sit behind some windows and the Roamie cannot follow the cursor everywhere.', optional: true }),
    });
  }

  const pw = await run('pipewire', ['--version']);
  const pwv = /(\d+)\.(\d+)\.(\d+)/.exec(pw ?? '');
  const pwOk = !!pwv && (+pwv[1] > 1 || (+pwv[1] === 1 && (+pwv[2] > 4 || (+pwv[2] === 4 && +pwv[3] >= 8))));
  if (session === 'wayland') out.push({
    id: 'pipewire', name: 'PipeWire 1.4.8 or newer (cursor following)', ok: pwOk, optional: true,
    what: 'Only for the Roamie following your cursor on niri. Without it Roamies works, but it cannot see the pointer outside its own windows.',
    ...(pwOk ? {} : { note: pwv ? `You have ${pwv[0]}. Newer PipeWire comes with a newer distro release.` : 'PipeWire was not found.' }),
  });

  const xo = await has('xdg-open');
  out.push({
    id: 'xdg-open', name: 'xdg-open (opening links and folders)', ok: xo, optional: true,
    what: 'Used to open web pages and the download page for updates.',
    ...(xo ? {} : { fix: installLine(osRelease, { apt: ['xdg-utils'], dnf: ['xdg-utils'], pacman: ['xdg-utils'], zypper: ['xdg-utils'] }) ?? 'Install xdg-utils.' }),
  });
  return out;
}
