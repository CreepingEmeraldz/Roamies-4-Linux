// Pre-read: a background read of the front window at the start of a request is used once by the next read_window,
// only while fresh, and not at all after something that could change the screen.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

if (process.platform !== 'linux') { console.log('preread: Linux only, skipped'); process.exit(0); }
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'preread-'));
const log = path.join(dir, 'log');
const fake = path.join(dir, 'atspi');
fs.writeFileSync(fake, `#!/bin/sh
echo "$1" >> ${log}
case "$1" in
  ui) echo '{"ok":true,"window":{"id":"1","app":"Brave","title":"Page"},"controls":[{"id":"c1","role":"link","label":"View source"}],"texts":["hello"],"note":""}' ;;
  windows) echo '{"ok":true,"windows":[{"id":"1","app":"Brave","title":"Page"}]}' ;;
  *) echo '{"ok":true}' ;;
esac
`, { mode: 0o755 });
process.env.ROAMIES_ATSPI = fake;
delete process.env.ROAMIES_HANDS;

const { createTools } = await import('../tools.mjs');
const noop = () => {};
const made = createTools({ allowActions: () => true, ui: noop, ask: async () => true, openUrl: noop, onCommitments: noop, showList: noop, fmtWhen: String, commitmentsText: () => '', userName: 'T' });
const read = () => made.tools.find((t) => t.name === 'read_window').execute({ target: 'front' }, { toolCallId: 'x' });
const uis = () => (fs.existsSync(log) ? fs.readFileSync(log, 'utf8').split('\n').filter((l) => l === 'ui').length : 0);

const first = await read();
assert.match(first, /View source/);
assert.equal(uis(), 1, 'no pre-read: one read of the window');

made.prefetchFront();
await new Promise((r) => setTimeout(r, 300));
assert.equal(uis(), 2, 'the pre-read ran in the background');
assert.match(await read(), /View source/);
assert.equal(uis(), 2, 'the first read used the pre-read, with no new read of the window');
await read();
assert.equal(uis(), 3, 'a pre-read is used once');

made.prefetchFront();
await new Promise((r) => setTimeout(r, 300));
await made.tools.find((t) => t.name === 'run_command').execute({ command: 'true' }, { toolCallId: 'y' }).catch(() => {});
await read();
assert.equal(uis(), 5, 'a command in between drops the pre-read, so the read is fresh');
console.log('preread ok');
