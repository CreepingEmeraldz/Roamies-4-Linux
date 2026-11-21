import test from 'node:test';
import assert from 'node:assert/strict';
import { checkDeps, installLine, portalPackages } from '../deps.mjs';

const ZORIN = 'NAME="Zorin OS"\nID=zorin\nID_LIKE="ubuntu debian"\n';
const FEDORA = 'ID=fedora\n';

test('the install line follows the distro family', () => {
  const n = { apt: ['tesseract-ocr'], dnf: ['tesseract'], pacman: ['tesseract'] };
  assert.equal(installLine(ZORIN, n), 'sudo apt install tesseract-ocr');
  assert.equal(installLine(FEDORA, n), 'sudo dnf install tesseract');
  assert.equal(installLine('ID=arch\n', n), 'sudo pacman -S tesseract');
  assert.equal(installLine('ID=nixos\n', n), null);
  assert.equal(installLine('', n), null);
});
test('the portal backend follows the desktop', () => {
  assert.deepEqual(portalPackages('niri').apt, ['xdg-desktop-portal-gnome']);
  assert.deepEqual(portalPackages('sway').apt, ['xdg-desktop-portal-wlr']);
  assert.deepEqual(portalPackages('Hyprland').apt, ['xdg-desktop-portal-hyprland']);
  assert.deepEqual(portalPackages('KDE').apt, ['xdg-desktop-portal-kde']);
});
test('a missing tesseract is reported with the fix, a present one is not', async () => {
  const none = async () => null;
  const a = await checkDeps({ overlay: 'layer', session: 'wayland', desktop: 'niri', osRelease: ZORIN, run: none });
  const t = a.find((d) => d.id === 'tesseract');
  assert.equal(t.ok, false);
  assert.equal(t.fix, 'sudo apt install tesseract-ocr');
  const have = async (cmd, args) => (cmd === 'tesseract' ? 'List of available languages (2):\neng\nosd\n' : cmd === 'sh' ? 'x' : cmd === 'gdbus' ? '(true,)' : cmd === 'pipewire' ? 'pipewire\nCompiled with libpipewire 1.0.5' : null);
  const b = await checkDeps({ overlay: 'layer', session: 'wayland', desktop: 'niri', osRelease: ZORIN, run: have });
  assert.equal(b.find((d) => d.id === 'tesseract').ok, true);
  assert.equal(b.find((d) => d.id === 'tesseract').fix, undefined);
  assert.equal(b.find((d) => d.id === 'layer').ok, true);
  assert.equal(b.find((d) => d.id === 'portal').ok, true);
  assert.equal(b.find((d) => d.id === 'pipewire').ok, false, '1.0.5 is too old for the pointer helper');
});
test('layer-shell off is reported as optional, with no install line; X11 has no portal or layer rows', async () => {
  const run = async () => 'x';
  const a = await checkDeps({ overlay: 'xwayland', session: 'wayland', desktop: 'GNOME', osRelease: ZORIN, run });
  const l = a.find((d) => d.id === 'layer');
  assert.equal(l.ok, false); assert.equal(l.optional, true); assert.equal(l.fix, undefined);
  const x = await checkDeps({ overlay: 'x11', session: 'x11', desktop: 'XFCE', osRelease: ZORIN, run });
  assert.equal(x.some((d) => d.id === 'portal' || d.id === 'layer'), false);
});
